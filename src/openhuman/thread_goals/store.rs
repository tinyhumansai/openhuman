//! Persistence for the thread-level goal.
//!
//! Each thread's goal lives in a single JSON file at
//! `<workspace>/thread_goals/<hex(thread_id)>.json` — the same per-thread
//! file-JSON pattern as the kanban task board
//! ([`crate::openhuman::agent::task_board`]). There is at most one goal per
//! thread.
//!
//! All mutations go through a load → mutate → atomic-persist sequence,
//! serialised by a process-wide mutex so concurrent writers (agent tools, RPC,
//! the heartbeat continuation runtime, and post-turn accounting) can't clobber
//! each other.

use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use chrono::Utc;
use tokio::sync::Mutex;

use super::types::{ThreadGoal, ThreadGoalStatus};

const THREAD_GOALS_DIR: &str = "thread_goals";
const THREAD_GOALS_EXTENSION: &str = "json";

/// Serialises load→mutate→save sequences across all callers.
fn goal_mutation_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Current unix time in milliseconds.
fn now_ms() -> u64 {
    Utc::now().timestamp_millis().max(0) as u64
}

fn validate_thread_id(thread_id: &str) -> Result<String, String> {
    let trimmed = thread_id.trim();
    if trimmed.is_empty() {
        return Err("invalid thread goal thread_id: empty or whitespace".to_string());
    }
    Ok(trimmed.to_string())
}

/// On-disk store rooted at the user's `workspace_dir`.
#[derive(Debug, Clone)]
pub struct ThreadGoalStore {
    workspace_dir: PathBuf,
}

impl ThreadGoalStore {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }

    fn ensure_dir(&self) -> Result<PathBuf, String> {
        let dir = self.workspace_dir.join(THREAD_GOALS_DIR);
        fs::create_dir_all(&dir)
            .map_err(|e| format!("create thread goals dir {}: {e}", dir.display()))?;
        Ok(dir)
    }

    fn goal_path(&self, thread_id: &str) -> Result<PathBuf, String> {
        let thread_id = validate_thread_id(thread_id)?;
        Ok(self.workspace_dir.join(THREAD_GOALS_DIR).join(format!(
            "{}.{}",
            hex::encode(thread_id.as_bytes()),
            THREAD_GOALS_EXTENSION
        )))
    }

    /// Load the goal for `thread_id`, or `None` if the thread has no goal yet.
    pub fn get(&self, thread_id: &str) -> Result<Option<ThreadGoal>, String> {
        let thread_id = validate_thread_id(thread_id)?;
        tracing::debug!(thread_id = %thread_id, "[thread_goals] get entry");
        let path = self.goal_path(&thread_id)?;
        if !path.exists() {
            tracing::debug!(thread_id = %thread_id, "[thread_goals] get not_found");
            return Ok(None);
        }
        let mut buf = String::new();
        fs::File::open(&path)
            .map_err(|e| format!("open thread goal {}: {e}", path.display()))?
            .read_to_string(&mut buf)
            .map_err(|e| format!("read thread goal {}: {e}", path.display()))?;
        let goal = serde_json::from_str::<ThreadGoal>(&buf).map_err(|e| {
            format!(
                "parse thread goal {} for thread '{}': {e}",
                path.display(),
                thread_id
            )
        })?;
        tracing::debug!(
            thread_id = %thread_id,
            goal_id = %goal.goal_id,
            status = goal.status.as_str(),
            "[thread_goals] get ok"
        );
        Ok(Some(goal))
    }

    /// Persist `goal` atomically (temp file + fsync + rename), mirroring the
    /// task board's durable write path.
    fn put(&self, goal: &ThreadGoal) -> Result<(), String> {
        let thread_id = validate_thread_id(&goal.thread_id)?;
        let dir = self.ensure_dir()?;
        let path = self.goal_path(&thread_id)?;
        let mut tmp = tempfile::NamedTempFile::new_in(&dir)
            .map_err(|e| format!("create thread goal tempfile in {}: {e}", dir.display()))?;
        let bytes =
            serde_json::to_vec_pretty(goal).map_err(|e| format!("serialize thread goal: {e}"))?;
        tmp.write_all(&bytes)
            .map_err(|e| format!("write thread goal tempfile: {e}"))?;
        tmp.as_file()
            .sync_all()
            .map_err(|e| format!("fsync thread goal tempfile: {e}"))?;
        tmp.persist(&path)
            .map_err(|e| format!("persist thread goal {}: {e}", path.display()))?;
        tracing::debug!(
            thread_id = %thread_id,
            goal_id = %goal.goal_id,
            status = goal.status.as_str(),
            tokens_used = goal.tokens_used,
            "[thread_goals] put ok"
        );
        Ok(())
    }

    /// Delete the goal for `thread_id`. Returns whether a file was removed.
    fn delete_file(&self, thread_id: &str) -> Result<bool, String> {
        let path = self.goal_path(thread_id)?;
        if !path.exists() {
            return Ok(false);
        }
        fs::remove_file(&path)
            .map_err(|e| format!("delete thread goal {}: {e}", path.display()))?;
        Ok(true)
    }
}

// ── Async mutation API (mutex-guarded load→mutate→save) ──────────────────────

/// Create or replace the goal for `thread_id`.
///
/// Codex semantics: if `objective` **differs** from the current one, a fresh
/// `goal_id` is minted and the usage counters reset (status → `Active`). If the
/// objective is **unchanged**, counters and `goal_id` are preserved and only
/// the budget / `updated_at` are refreshed (status returns to `Active` so a
/// completed/paused goal can be explicitly re-opened by re-setting it).
pub async fn set(
    workspace_dir: &Path,
    thread_id: &str,
    objective: &str,
    token_budget: Option<u64>,
) -> Result<ThreadGoal, String> {
    let objective = objective.trim();
    if objective.is_empty() {
        return Err("thread goal objective must not be empty".to_string());
    }
    let thread_id = validate_thread_id(thread_id)?;
    let _guard = goal_mutation_lock().lock().await;
    let store = ThreadGoalStore::new(workspace_dir.to_path_buf());
    let goal = compute_and_put_set(&store, &thread_id, objective, token_budget)?;
    tracing::info!(
        thread_id = %thread_id,
        goal_id = %goal.goal_id,
        "[thread_goals] set objective ({} chars), budget={:?}",
        goal.objective.chars().count(),
        goal.token_budget
    );
    Ok(goal)
}

/// Build + persist the goal for a `set`. The caller MUST hold
/// [`goal_mutation_lock`] so the read-modify-write is atomic.
fn compute_and_put_set(
    store: &ThreadGoalStore,
    thread_id: &str,
    objective: &str,
    token_budget: Option<u64>,
) -> Result<ThreadGoal, String> {
    let now = now_ms();
    let goal = match store.get(thread_id)? {
        Some(mut existing) if existing.objective == objective => {
            // Same objective → preserve counters/goal_id; refresh budget.
            existing.token_budget = token_budget;
            existing.continuation_suppressed = false;
            existing.updated_at_ms = now;
            // Re-open to Active, but don't un-limit a goal that is still over its
            // (possibly updated) budget — counters are only reset by a *changed*
            // objective, so a same-objective re-set must stay budget_limited.
            existing.status = if existing.over_budget() {
                ThreadGoalStatus::BudgetLimited
            } else {
                ThreadGoalStatus::Active
            };
            existing
        }
        existing => {
            // New / changed objective → fresh goal_id, reset counters.
            let created_at_ms = existing.as_ref().map(|g| g.created_at_ms).unwrap_or(now);
            ThreadGoal {
                thread_id: thread_id.to_string(),
                goal_id: uuid::Uuid::new_v4().to_string(),
                objective: objective.to_string(),
                status: ThreadGoalStatus::Active,
                token_budget,
                tokens_used: 0,
                time_used_seconds: 0,
                created_at_ms,
                updated_at_ms: now,
                continuation_suppressed: false,
            }
        }
    };
    store.put(&goal)?;
    Ok(goal)
}

/// Set the goal **only if the thread has none yet**. Returns `Some(goal)` when a
/// new goal was created, or `None` when a goal already existed (left untouched).
///
/// This backs the "scout proposes if empty, orchestrator authoritative"
/// precedence: the context-gathering path may bootstrap a goal on the first
/// turn but must never clobber a user/orchestrator-refined one. The check and
/// the write run under a single lock acquisition so a concurrent `set` /
/// `set_if_absent` can't slip into the gap and get clobbered.
pub async fn set_if_absent(
    workspace_dir: &Path,
    thread_id: &str,
    objective: &str,
    token_budget: Option<u64>,
) -> Result<Option<ThreadGoal>, String> {
    let objective = objective.trim();
    if objective.is_empty() {
        return Err("thread goal objective must not be empty".to_string());
    }
    let thread_id = validate_thread_id(thread_id)?;
    let _guard = goal_mutation_lock().lock().await;
    let store = ThreadGoalStore::new(workspace_dir.to_path_buf());
    if store.get(&thread_id)?.is_some() {
        tracing::debug!(thread_id = %thread_id, "[thread_goals] set_if_absent skipped (exists)");
        return Ok(None);
    }
    let goal = compute_and_put_set(&store, &thread_id, objective, token_budget)?;
    Ok(Some(goal))
}

/// Load the goal for `thread_id` (read-only), or `None`.
pub async fn get(workspace_dir: &Path, thread_id: &str) -> Result<Option<ThreadGoal>, String> {
    let store = ThreadGoalStore::new(workspace_dir.to_path_buf());
    store.get(thread_id)
}

/// Load every persisted thread goal (read-only). Skips files that fail to parse
/// (logged) so one corrupt entry can't hide the rest. Used by the heartbeat
/// continuation runtime to find idle candidates.
pub async fn list_all(workspace_dir: &Path) -> Result<Vec<ThreadGoal>, String> {
    let dir = workspace_dir.join(THREAD_GOALS_DIR);
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("read thread goals dir {}: {e}", dir.display())),
    };
    let mut goals = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("iterate thread goals dir: {e}"))?
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some(THREAD_GOALS_EXTENSION) {
            continue;
        }
        match tokio::fs::read_to_string(&path).await {
            Ok(body) => match serde_json::from_str::<ThreadGoal>(&body) {
                Ok(goal) => goals.push(goal),
                Err(e) => {
                    tracing::debug!(path = %path.display(), error = %e, "[thread_goals] list_all skip parse error");
                }
            },
            Err(e) => {
                tracing::debug!(path = %path.display(), error = %e, "[thread_goals] list_all skip read error");
            }
        }
    }
    Ok(goals)
}

/// Delete the goal for `thread_id`. Returns whether one existed.
pub async fn clear(workspace_dir: &Path, thread_id: &str) -> Result<bool, String> {
    let _guard = goal_mutation_lock().lock().await;
    let store = ThreadGoalStore::new(workspace_dir.to_path_buf());
    let removed = store.delete_file(thread_id)?;
    tracing::info!(thread_id = %thread_id, removed, "[thread_goals] clear");
    Ok(removed)
}

/// Generic guarded mutator: load, apply `f`, persist. Returns the updated goal,
/// or an error if the thread has no goal.
async fn mutate<F>(workspace_dir: &Path, thread_id: &str, f: F) -> Result<ThreadGoal, String>
where
    F: FnOnce(&mut ThreadGoal),
{
    let thread_id = validate_thread_id(thread_id)?;
    let _guard = goal_mutation_lock().lock().await;
    let store = ThreadGoalStore::new(workspace_dir.to_path_buf());
    let mut goal = store
        .get(&thread_id)?
        .ok_or_else(|| format!("no thread goal for thread '{thread_id}'"))?;
    f(&mut goal);
    goal.updated_at_ms = now_ms();
    store.put(&goal)?;
    Ok(goal)
}

/// Mark the goal `Complete` (model-driven success).
pub async fn complete(workspace_dir: &Path, thread_id: &str) -> Result<ThreadGoal, String> {
    let goal = mutate(workspace_dir, thread_id, |g| {
        g.status = ThreadGoalStatus::Complete;
        g.continuation_suppressed = true;
    })
    .await?;
    tracing::info!(thread_id = %thread_id, goal_id = %goal.goal_id, "[thread_goals] complete");
    Ok(goal)
}

/// Pause an `Active` goal (system-driven, on interrupt/abort). A no-op for goals
/// that aren't currently active.
pub async fn pause(workspace_dir: &Path, thread_id: &str) -> Result<ThreadGoal, String> {
    let goal = mutate(workspace_dir, thread_id, |g| {
        if g.status.is_active() {
            g.status = ThreadGoalStatus::Paused;
        }
    })
    .await?;
    tracing::info!(thread_id = %thread_id, status = goal.status.as_str(), "[thread_goals] pause");
    Ok(goal)
}

/// Resume a `Paused` goal (system-driven, on thread resume). A no-op for goals
/// that aren't paused (a completed/budget-limited goal is not reactivated).
pub async fn resume(workspace_dir: &Path, thread_id: &str) -> Result<ThreadGoal, String> {
    let goal = mutate(workspace_dir, thread_id, |g| {
        if matches!(g.status, ThreadGoalStatus::Paused) {
            g.status = ThreadGoalStatus::Active;
            g.continuation_suppressed = false;
        }
    })
    .await?;
    tracing::info!(thread_id = %thread_id, status = goal.status.as_str(), "[thread_goals] resume");
    Ok(goal)
}

/// Set or clear the `continuation_suppressed` flag (idle-continuation guard).
pub async fn set_continuation_suppressed(
    workspace_dir: &Path,
    thread_id: &str,
    suppressed: bool,
) -> Result<ThreadGoal, String> {
    mutate(workspace_dir, thread_id, |g| {
        g.continuation_suppressed = suppressed;
    })
    .await
}

/// Set `continuation_suppressed` only when the thread's current goal still
/// matches `expected_goal_id` (compare-and-set). Returns the goal as it stands
/// after the (possibly skipped) write, or `None` when the thread has no goal.
///
/// Used by the continuation runtime so a goal that was completed or replaced
/// during the autonomous turn is never suppressed by the post-dispatch write.
pub async fn set_continuation_suppressed_if(
    workspace_dir: &Path,
    thread_id: &str,
    expected_goal_id: &str,
    suppressed: bool,
) -> Result<Option<ThreadGoal>, String> {
    let thread_id = validate_thread_id(thread_id)?;
    let _guard = goal_mutation_lock().lock().await;
    let store = ThreadGoalStore::new(workspace_dir.to_path_buf());
    let Some(mut goal) = store.get(&thread_id)? else {
        return Ok(None);
    };
    // Skip when the goal was replaced (goal_id mismatch), is no longer active
    // (completed/paused/budget_limited during the turn — must not be re-touched),
    // or is already in the requested state.
    if goal.goal_id != expected_goal_id
        || !goal.status.is_active()
        || goal.continuation_suppressed == suppressed
    {
        return Ok(Some(goal));
    }
    goal.continuation_suppressed = suppressed;
    goal.updated_at_ms = now_ms();
    store.put(&goal)?;
    Ok(Some(goal))
}

/// Account token + time usage against the goal, applying the budget constraint.
///
/// **Stale-write guard (Codex parity):** the delta is **silently ignored** when
/// `expected_goal_id` doesn't match the current goal — an in-flight accounting
/// call from a now-replaced goal must not corrupt the new one. Returns the goal
/// as it stands after the (possibly skipped) update, or `None` if there is no
/// goal for the thread.
pub async fn account_usage(
    workspace_dir: &Path,
    thread_id: &str,
    expected_goal_id: &str,
    token_delta: u64,
    secs_delta: u64,
) -> Result<Option<ThreadGoal>, String> {
    let thread_id = validate_thread_id(thread_id)?;
    let _guard = goal_mutation_lock().lock().await;
    let store = ThreadGoalStore::new(workspace_dir.to_path_buf());
    let Some(mut goal) = store.get(&thread_id)? else {
        return Ok(None);
    };
    if goal.goal_id != expected_goal_id {
        tracing::debug!(
            thread_id = %thread_id,
            expected = %expected_goal_id,
            actual = %goal.goal_id,
            "[thread_goals] account_usage ignored stale goal_id"
        );
        return Ok(Some(goal));
    }
    if token_delta == 0 && secs_delta == 0 {
        return Ok(Some(goal));
    }
    goal.tokens_used = goal.tokens_used.saturating_add(token_delta);
    goal.time_used_seconds = goal.time_used_seconds.saturating_add(secs_delta);
    // Apply the budget cap: an active goal that crosses its budget becomes
    // budget-limited. Completed/paused goals keep their status.
    if goal.status.is_active() && goal.over_budget() {
        goal.status = ThreadGoalStatus::BudgetLimited;
        tracing::info!(
            thread_id = %thread_id,
            goal_id = %goal.goal_id,
            tokens_used = goal.tokens_used,
            budget = ?goal.token_budget,
            "[thread_goals] budget reached → budget_limited"
        );
    }
    goal.updated_at_ms = now_ms();
    store.put(&goal)?;
    Ok(Some(goal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn set_get_clear_round_trip() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        assert!(get(dir, "t1").await.unwrap().is_none());

        let g = set(dir, "t1", "ship the feature", None).await.unwrap();
        assert_eq!(g.objective, "ship the feature");
        assert_eq!(g.status, ThreadGoalStatus::Active);
        assert_eq!(g.tokens_used, 0);

        let loaded = get(dir, "t1").await.unwrap().unwrap();
        assert_eq!(loaded.goal_id, g.goal_id);

        assert!(clear(dir, "t1").await.unwrap());
        assert!(get(dir, "t1").await.unwrap().is_none());
        assert!(!clear(dir, "t1").await.unwrap());
    }

    #[tokio::test]
    async fn set_same_objective_preserves_goal_id_and_counters() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        let g1 = set(dir, "t", "objective A", Some(100)).await.unwrap();
        account_usage(dir, "t", &g1.goal_id, 30, 5)
            .await
            .unwrap()
            .unwrap();
        let g2 = set(dir, "t", "objective A", Some(200)).await.unwrap();
        assert_eq!(g1.goal_id, g2.goal_id, "same objective keeps goal_id");
        assert_eq!(g2.tokens_used, 30, "counters preserved");
        assert_eq!(g2.token_budget, Some(200), "budget refreshed");
    }

    #[tokio::test]
    async fn set_same_objective_stays_budget_limited_when_over_budget() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        let g = set(dir, "t", "obj", Some(100)).await.unwrap();
        // Burn past the budget → BudgetLimited.
        let limited = account_usage(dir, "t", &g.goal_id, 120, 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(limited.status, ThreadGoalStatus::BudgetLimited);
        // Re-setting the SAME objective preserves counters, so it must NOT
        // silently re-activate while still over budget.
        let resed = set(dir, "t", "obj", Some(100)).await.unwrap();
        assert_eq!(resed.tokens_used, 120, "same-objective preserves counters");
        assert_eq!(
            resed.status,
            ThreadGoalStatus::BudgetLimited,
            "still over budget → must stay budget_limited, not active"
        );
        // Raising the budget above usage re-opens it.
        let raised = set(dir, "t", "obj", Some(1000)).await.unwrap();
        assert_eq!(raised.status, ThreadGoalStatus::Active);
    }

    #[tokio::test]
    async fn set_changed_objective_mints_new_goal_id_and_resets() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        let g1 = set(dir, "t", "objective A", None).await.unwrap();
        account_usage(dir, "t", &g1.goal_id, 30, 5)
            .await
            .unwrap()
            .unwrap();
        let g2 = set(dir, "t", "objective B", None).await.unwrap();
        assert_ne!(g1.goal_id, g2.goal_id);
        assert_eq!(g2.tokens_used, 0, "counters reset on new objective");
        assert_eq!(g2.created_at_ms, g1.created_at_ms, "created_at preserved");
    }

    #[tokio::test]
    async fn set_if_absent_only_bootstraps_when_empty() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        let created = set_if_absent(dir, "t", "scout goal", None).await.unwrap();
        assert!(created.is_some());
        // Second call must not clobber.
        let again = set_if_absent(dir, "t", "different goal", None)
            .await
            .unwrap();
        assert!(again.is_none());
        assert_eq!(
            get(dir, "t").await.unwrap().unwrap().objective,
            "scout goal"
        );
    }

    #[tokio::test]
    async fn account_usage_ignores_stale_goal_id() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        let g = set(dir, "t", "obj", None).await.unwrap();
        // Stale id → ignored, counters unchanged.
        let after = account_usage(dir, "t", "not-the-goal-id", 50, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.tokens_used, 0);
        // Correct id → applied.
        let after = account_usage(dir, "t", &g.goal_id, 50, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.tokens_used, 50);
    }

    #[tokio::test]
    async fn account_usage_trips_budget_limited() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        let g = set(dir, "t", "obj", Some(100)).await.unwrap();
        let after = account_usage(dir, "t", &g.goal_id, 120, 2)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.status, ThreadGoalStatus::BudgetLimited);
        assert_eq!(after.budget_remaining(), Some(0));
    }

    #[tokio::test]
    async fn pause_resume_complete_transitions() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        set(dir, "t", "obj", None).await.unwrap();
        assert_eq!(
            pause(dir, "t").await.unwrap().status,
            ThreadGoalStatus::Paused
        );
        assert_eq!(
            resume(dir, "t").await.unwrap().status,
            ThreadGoalStatus::Active
        );
        let done = complete(dir, "t").await.unwrap();
        assert_eq!(done.status, ThreadGoalStatus::Complete);
        // Resume does not reactivate a completed goal.
        assert_eq!(
            resume(dir, "t").await.unwrap().status,
            ThreadGoalStatus::Complete
        );
    }

    #[tokio::test]
    async fn mutators_error_without_a_goal() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        assert!(complete(dir, "missing").await.is_err());
        assert!(pause(dir, "missing").await.is_err());
    }

    #[tokio::test]
    async fn empty_objective_and_blank_thread_id_rejected() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        assert!(set(dir, "t", "   ", None).await.is_err());
        assert!(set(dir, "  ", "obj", None).await.is_err());
    }
}
