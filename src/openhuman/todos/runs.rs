//! Durable task-run records with heartbeat liveness and stale reclaim.
//!
//! Each time the [`crate::openhuman::agent::task_dispatcher`] claims a card,
//! it creates a [`TaskRun`] that tracks: who claimed it, when, last heartbeat,
//! completion outcome, and error/evidence. A background heartbeat timer ticks
//! alongside the autonomous run so healthy long-running workers stay live while
//! wedged workers can be detected and reclaimed.
//!
//! Stale reclaim policy: a run whose heartbeat is older than
//! [`RunLimits::heartbeat_stale_secs`] **or** whose total age exceeds
//! [`RunLimits::claim_ttl_secs`] is eligible for reclaim. Reclaimed cards move
//! back to `todo` (re-dispatchable) unless they've been reclaimed more than
//! [`RunLimits::max_reclaim_count`] times, in which case they park as `blocked`
//! with a diagnostic blocker message.

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::openhuman::agent::task_board::TaskCardStatus;

use super::ops::{self, BoardLocation, CardPatch};

// ── Defaults ───────────────────────────────────────────────────────────

pub const DEFAULT_HEARTBEAT_STALE_SECS: u64 = 300;
pub const DEFAULT_CLAIM_TTL_SECS: u64 = 3600;
pub const DEFAULT_MAX_RECLAIM_COUNT: u32 = 3;
const HEARTBEAT_TICK_SECS: u64 = 30;

// ── Types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    Success,
    Failed,
    Reclaimed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRun {
    pub run_id: String,
    pub card_id: String,
    pub claimed_by: String,
    pub claim_token: String,
    pub started_at: String,
    pub last_heartbeat_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<RunOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
}

impl TaskRun {
    pub fn is_active(&self) -> bool {
        self.completed_at.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLimits {
    pub heartbeat_stale_secs: u64,
    pub claim_ttl_secs: u64,
    pub max_reclaim_count: u32,
}

impl Default for RunLimits {
    fn default() -> Self {
        Self {
            heartbeat_stale_secs: DEFAULT_HEARTBEAT_STALE_SECS,
            claim_ttl_secs: DEFAULT_CLAIM_TTL_SECS,
            max_reclaim_count: DEFAULT_MAX_RECLAIM_COUNT,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReclaimResult {
    pub reclaimed_count: usize,
    pub blocked_count: usize,
    pub details: Vec<ReclaimDetail>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReclaimDetail {
    pub run_id: String,
    pub card_id: String,
    pub reason: String,
    pub new_card_status: String,
}

// ── Per-board lock for run records ─────────────────────────────────────

fn run_lock(location: &BoardLocation) -> Arc<Mutex<()>> {
    static MAP: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let map_mu = MAP.get_or_init(|| Mutex::new(HashMap::new()));
    let key = match location {
        BoardLocation::Thread { thread_id, .. } => format!("runs:{thread_id}"),
        BoardLocation::Scratch => "runs:_scratch_".to_string(),
    };
    map_mu.lock().entry(key).or_default().clone()
}

// ── Store ──────────────────────────────────────────────────────────────

const TASK_BOARD_DIR: &str = "agent_task_boards";

fn runs_path(workspace_dir: &Path, thread_id: &str) -> PathBuf {
    workspace_dir
        .join(TASK_BOARD_DIR)
        .join(format!("{}.runs.json", hex::encode(thread_id.as_bytes())))
}

fn load_runs(location: &BoardLocation) -> Result<Vec<TaskRun>, String> {
    let BoardLocation::Thread {
        workspace_dir,
        thread_id,
    } = location
    else {
        return Ok(Vec::new());
    };
    let path = runs_path(workspace_dir, thread_id);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut buf = String::new();
    fs::File::open(&path)
        .map_err(|e| format!("open runs {}: {e}", path.display()))?
        .read_to_string(&mut buf)
        .map_err(|e| format!("read runs {}: {e}", path.display()))?;
    serde_json::from_str::<Vec<TaskRun>>(&buf)
        .map_err(|e| format!("parse runs {}: {e}", path.display()))
}

fn save_runs(location: &BoardLocation, runs: &[TaskRun]) -> Result<(), String> {
    let BoardLocation::Thread {
        workspace_dir,
        thread_id,
    } = location
    else {
        return Ok(());
    };
    let dir = workspace_dir.join(TASK_BOARD_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("create runs dir {}: {e}", dir.display()))?;
    let path = runs_path(workspace_dir, thread_id);
    let bytes = serde_json::to_vec_pretty(&runs).map_err(|e| format!("serialize runs: {e}"))?;
    let mut tmp =
        tempfile::NamedTempFile::new_in(&dir).map_err(|e| format!("create runs tempfile: {e}"))?;
    tmp.write_all(&bytes)
        .map_err(|e| format!("write runs tempfile: {e}"))?;
    tmp.as_file()
        .sync_all()
        .map_err(|e| format!("fsync runs tempfile: {e}"))?;
    tmp.persist(&path)
        .map_err(|e| format!("persist runs {}: {e}", path.display()))?;
    Ok(())
}

// ── Operations ─────────────────────────────────────────────────────────

pub fn create_run(
    location: &BoardLocation,
    run_id: &str,
    card_id: &str,
    claimed_by: &str,
) -> Result<TaskRun, String> {
    let lock = run_lock(location);
    let _guard = lock.lock();

    let now = Utc::now().to_rfc3339();
    let claim_token = uuid::Uuid::new_v4().to_string();

    tracing::debug!(
        run_id = %run_id,
        card_id = %card_id,
        claimed_by = %claimed_by,
        "[todos][runs] create_run entry"
    );

    let run = TaskRun {
        run_id: run_id.to_string(),
        card_id: card_id.to_string(),
        claimed_by: claimed_by.to_string(),
        claim_token: claim_token.clone(),
        started_at: now.clone(),
        last_heartbeat_at: now,
        completed_at: None,
        outcome: None,
        error: None,
        evidence: Vec::new(),
    };

    let mut runs = load_runs(location)?;
    runs.push(run.clone());
    save_runs(location, &runs)?;

    tracing::info!(
        run_id = %run_id,
        card_id = %card_id,
        claim_token = %claim_token,
        "[todos][runs] create_run ok"
    );
    Ok(run)
}

pub fn update_heartbeat(location: &BoardLocation, run_id: &str) -> Result<(), String> {
    let lock = run_lock(location);
    let _guard = lock.lock();

    let mut runs = load_runs(location)?;
    let run = runs
        .iter_mut()
        .find(|r| r.run_id == run_id && r.is_active())
        .ok_or_else(|| format!("[todos][runs] active run '{run_id}' not found for heartbeat"))?;

    run.last_heartbeat_at = Utc::now().to_rfc3339();
    save_runs(location, &runs)?;

    tracing::trace!(
        run_id = %run_id,
        "[todos][runs] heartbeat updated"
    );
    Ok(())
}

pub fn complete_run(
    location: &BoardLocation,
    run_id: &str,
    outcome: RunOutcome,
    error: Option<String>,
    evidence: Vec<String>,
) -> Result<TaskRun, String> {
    let lock = run_lock(location);
    let _guard = lock.lock();

    tracing::debug!(
        run_id = %run_id,
        outcome = ?outcome,
        "[todos][runs] complete_run entry"
    );

    let mut runs = load_runs(location)?;
    let run = runs
        .iter_mut()
        .find(|r| r.run_id == run_id && r.is_active())
        .ok_or_else(|| format!("[todos][runs] active run '{run_id}' not found for completion"))?;

    run.completed_at = Some(Utc::now().to_rfc3339());
    run.outcome = Some(outcome);
    run.error = error;
    run.evidence = evidence;
    let completed = run.clone();

    save_runs(location, &runs)?;

    tracing::info!(
        run_id = %run_id,
        outcome = ?completed.outcome,
        "[todos][runs] complete_run ok"
    );
    Ok(completed)
}

pub fn list_runs(location: &BoardLocation, card_id: Option<&str>) -> Result<Vec<TaskRun>, String> {
    let lock = run_lock(location);
    let _guard = lock.lock();

    let runs = load_runs(location)?;
    Ok(match card_id {
        Some(cid) => runs.into_iter().filter(|r| r.card_id == cid).collect(),
        None => runs,
    })
}

pub fn get_run(location: &BoardLocation, run_id: &str) -> Result<Option<TaskRun>, String> {
    let lock = run_lock(location);
    let _guard = lock.lock();

    let runs = load_runs(location)?;
    Ok(runs.into_iter().find(|r| r.run_id == run_id))
}

pub fn find_stale_runs(
    location: &BoardLocation,
    limits: &RunLimits,
) -> Result<Vec<(TaskRun, String)>, String> {
    let lock = run_lock(location);
    let _guard = lock.lock();

    let runs = load_runs(location)?;
    let now = Utc::now();
    let mut stale = Vec::new();

    for run in &runs {
        if !run.is_active() {
            continue;
        }
        if let Some(reason) = check_staleness(run, &now, limits) {
            stale.push((run.clone(), reason));
        }
    }
    Ok(stale)
}

fn check_staleness(run: &TaskRun, now: &DateTime<Utc>, limits: &RunLimits) -> Option<String> {
    let started: DateTime<Utc> = run.started_at.parse().ok()?;
    let last_hb: DateTime<Utc> = run.last_heartbeat_at.parse().ok()?;

    let age_secs = (*now - started).num_seconds().max(0) as u64;
    let hb_age_secs = (*now - last_hb).num_seconds().max(0) as u64;

    if age_secs > limits.claim_ttl_secs {
        return Some(format!(
            "claim TTL expired (age {age_secs}s > limit {}s)",
            limits.claim_ttl_secs
        ));
    }
    if hb_age_secs > limits.heartbeat_stale_secs {
        return Some(format!(
            "heartbeat stale (last heartbeat {hb_age_secs}s ago > limit {}s)",
            limits.heartbeat_stale_secs
        ));
    }
    None
}

/// Reclaim stale runs: mark the run as `Reclaimed`, then move the card
/// back to `todo` (re-dispatchable) or `blocked` (if reclaim count
/// exceeds `max_reclaim_count`).
pub fn reclaim_stale(
    location: &BoardLocation,
    limits: &RunLimits,
) -> Result<ReclaimResult, String> {
    tracing::debug!(
        thread_id = ?location.thread_id(),
        "[todos][runs] reclaim_stale entry"
    );

    let stale_runs = find_stale_runs(location, limits)?;
    if stale_runs.is_empty() {
        return Ok(ReclaimResult {
            reclaimed_count: 0,
            blocked_count: 0,
            details: Vec::new(),
        });
    }

    let mut reclaimed_count = 0usize;
    let mut blocked_count = 0usize;
    let mut details = Vec::new();

    for (stale_run, reason) in &stale_runs {
        if let Err(e) = complete_run(
            location,
            &stale_run.run_id,
            RunOutcome::Reclaimed,
            Some(reason.clone()),
            Vec::new(),
        ) {
            tracing::warn!(
                run_id = %stale_run.run_id,
                error = %e,
                "[todos][runs] failed to complete stale run"
            );
            continue;
        }

        let prior_reclaims = count_reclaims_for_card(location, &stale_run.card_id).unwrap_or(0);

        let (new_status, new_status_str) = if prior_reclaims >= limits.max_reclaim_count {
            (TaskCardStatus::Blocked, "blocked")
        } else {
            (TaskCardStatus::Todo, "todo")
        };

        let blocker_msg = if new_status == TaskCardStatus::Blocked {
            Some(format!(
                "Reclaimed {prior_reclaims} time(s), exceeding limit of {}. \
                 Last reclaim reason: {reason}",
                limits.max_reclaim_count
            ))
        } else {
            None
        };

        let patch = CardPatch {
            status: Some(new_status.clone()),
            blocker: blocker_msg,
            ..Default::default()
        };

        match ops::edit(location, &stale_run.card_id, patch) {
            Ok(_) => {
                tracing::info!(
                    run_id = %stale_run.run_id,
                    card_id = %stale_run.card_id,
                    new_status = new_status_str,
                    reason = %reason,
                    prior_reclaims,
                    "[todos][runs] card reclaimed"
                );

                if let Some(thread_id) = location.thread_id() {
                    crate::core::event_bus::publish_global(
                        crate::core::event_bus::DomainEvent::TaskRunReclaimed {
                            run_id: stale_run.run_id.clone(),
                            card_id: stale_run.card_id.clone(),
                            thread_id: thread_id.to_string(),
                            reason: reason.clone(),
                        },
                    );
                }

                if new_status == TaskCardStatus::Blocked {
                    blocked_count += 1;
                } else {
                    reclaimed_count += 1;
                }
                details.push(ReclaimDetail {
                    run_id: stale_run.run_id.clone(),
                    card_id: stale_run.card_id.clone(),
                    reason: reason.clone(),
                    new_card_status: new_status_str.to_string(),
                });
            }
            Err(e) => {
                tracing::warn!(
                    run_id = %stale_run.run_id,
                    card_id = %stale_run.card_id,
                    error = %e,
                    "[todos][runs] failed to update card after reclaim"
                );
            }
        }
    }

    tracing::info!(
        reclaimed_count,
        blocked_count,
        "[todos][runs] reclaim_stale complete"
    );

    Ok(ReclaimResult {
        reclaimed_count,
        blocked_count,
        details,
    })
}

fn count_reclaims_for_card(location: &BoardLocation, card_id: &str) -> Result<u32, String> {
    let runs = load_runs(location)?;
    let count = runs
        .iter()
        .filter(|r| r.card_id == card_id && r.outcome.as_ref() == Some(&RunOutcome::Reclaimed))
        .count();
    Ok(count as u32)
}

// ── Heartbeat background task ──────────────────────────────────────────

pub fn spawn_heartbeat_task(
    location: BoardLocation,
    run_id: String,
    cancel: tokio::sync::watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(HEARTBEAT_TICK_SECS));
        let mut cancel = cancel;
        ticker.tick().await; // skip the immediate fire
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if let Err(e) = update_heartbeat(&location, &run_id) {
                        tracing::debug!(
                            run_id = %run_id,
                            error = %e,
                            "[todos][runs] heartbeat tick failed (run may have completed)"
                        );
                        break;
                    }
                }
                _ = cancel.changed() => {
                    tracing::debug!(
                        run_id = %run_id,
                        "[todos][runs] heartbeat cancelled (run completed)"
                    );
                    break;
                }
            }
        }
    });
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn thread_loc(dir: &Path, id: &str) -> BoardLocation {
        BoardLocation::Thread {
            workspace_dir: dir.to_path_buf(),
            thread_id: id.to_string(),
        }
    }

    #[test]
    fn create_and_list_run() {
        let dir = tempdir().unwrap();
        let loc = thread_loc(dir.path(), "run-test-1");

        let run = create_run(&loc, "run-1", "card-1", "default").unwrap();
        assert_eq!(run.run_id, "run-1");
        assert_eq!(run.card_id, "card-1");
        assert_eq!(run.claimed_by, "default");
        assert!(run.is_active());
        assert!(!run.claim_token.is_empty());

        let all = list_runs(&loc, None).unwrap();
        assert_eq!(all.len(), 1);

        let by_card = list_runs(&loc, Some("card-1")).unwrap();
        assert_eq!(by_card.len(), 1);

        let empty = list_runs(&loc, Some("card-other")).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn heartbeat_updates_timestamp() {
        let dir = tempdir().unwrap();
        let loc = thread_loc(dir.path(), "hb-test-1");

        create_run(&loc, "run-hb", "card-1", "default").unwrap();
        let before = get_run(&loc, "run-hb").unwrap().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(10));
        update_heartbeat(&loc, "run-hb").unwrap();

        let after = get_run(&loc, "run-hb").unwrap().unwrap();
        assert!(after.last_heartbeat_at >= before.last_heartbeat_at);
    }

    #[test]
    fn heartbeat_fails_for_completed_run() {
        let dir = tempdir().unwrap();
        let loc = thread_loc(dir.path(), "hb-test-2");

        create_run(&loc, "run-done", "card-1", "default").unwrap();
        complete_run(&loc, "run-done", RunOutcome::Success, None, Vec::new()).unwrap();

        assert!(update_heartbeat(&loc, "run-done").is_err());
    }

    #[test]
    fn complete_run_sets_outcome() {
        let dir = tempdir().unwrap();
        let loc = thread_loc(dir.path(), "complete-test");

        create_run(&loc, "run-c", "card-1", "default").unwrap();
        let completed = complete_run(
            &loc,
            "run-c",
            RunOutcome::Success,
            None,
            vec!["opened PR #5".to_string()],
        )
        .unwrap();

        assert!(!completed.is_active());
        assert_eq!(completed.outcome, Some(RunOutcome::Success));
        assert!(completed.completed_at.is_some());
        assert_eq!(completed.evidence, vec!["opened PR #5"]);
    }

    #[test]
    fn complete_run_with_failure() {
        let dir = tempdir().unwrap();
        let loc = thread_loc(dir.path(), "fail-test");

        create_run(&loc, "run-f", "card-1", "default").unwrap();
        let completed = complete_run(
            &loc,
            "run-f",
            RunOutcome::Failed,
            Some("agent build failed".to_string()),
            Vec::new(),
        )
        .unwrap();

        assert_eq!(completed.outcome, Some(RunOutcome::Failed));
        assert_eq!(completed.error.as_deref(), Some("agent build failed"));
    }

    #[test]
    fn get_run_returns_none_for_missing() {
        let dir = tempdir().unwrap();
        let loc = thread_loc(dir.path(), "get-test");

        assert!(get_run(&loc, "no-such-run").unwrap().is_none());
    }

    #[test]
    fn check_staleness_detects_expired_ttl() {
        let now = Utc::now();
        let old = (now - chrono::Duration::seconds(7200)).to_rfc3339();
        let run = TaskRun {
            run_id: "r1".into(),
            card_id: "c1".into(),
            claimed_by: "test".into(),
            claim_token: "t".into(),
            started_at: old.clone(),
            last_heartbeat_at: now.to_rfc3339(),
            completed_at: None,
            outcome: None,
            error: None,
            evidence: Vec::new(),
        };
        let limits = RunLimits::default();
        let reason = check_staleness(&run, &now, &limits);
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("TTL expired"));
    }

    #[test]
    fn check_staleness_detects_stale_heartbeat() {
        let now = Utc::now();
        let recent_start = (now - chrono::Duration::seconds(60)).to_rfc3339();
        let old_hb = (now - chrono::Duration::seconds(600)).to_rfc3339();
        let run = TaskRun {
            run_id: "r2".into(),
            card_id: "c1".into(),
            claimed_by: "test".into(),
            claim_token: "t".into(),
            started_at: recent_start,
            last_heartbeat_at: old_hb,
            completed_at: None,
            outcome: None,
            error: None,
            evidence: Vec::new(),
        };
        let limits = RunLimits::default();
        let reason = check_staleness(&run, &now, &limits);
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("heartbeat stale"));
    }

    #[test]
    fn check_staleness_passes_healthy_run() {
        let now = Utc::now();
        let recent = (now - chrono::Duration::seconds(10)).to_rfc3339();
        let run = TaskRun {
            run_id: "r3".into(),
            card_id: "c1".into(),
            claimed_by: "test".into(),
            claim_token: "t".into(),
            started_at: recent.clone(),
            last_heartbeat_at: recent,
            completed_at: None,
            outcome: None,
            error: None,
            evidence: Vec::new(),
        };
        let limits = RunLimits::default();
        assert!(check_staleness(&run, &now, &limits).is_none());
    }

    #[test]
    fn reclaim_stale_moves_card_to_todo() {
        let dir = tempdir().unwrap();
        let loc = thread_loc(dir.path(), "reclaim-test-1");

        let snap = ops::add(&loc, "reclaimable task", CardPatch::default()).unwrap();
        let card_id = snap.cards[0].id.clone();
        ops::update_status(&loc, &card_id, TaskCardStatus::InProgress).unwrap();

        // Create a run with an old heartbeat
        {
            let lock = run_lock(&loc);
            let _guard = lock.lock();
            let old = (Utc::now() - chrono::Duration::seconds(600)).to_rfc3339();
            let run = TaskRun {
                run_id: "stale-run".into(),
                card_id: card_id.clone(),
                claimed_by: "test".into(),
                claim_token: "t".into(),
                started_at: old.clone(),
                last_heartbeat_at: old,
                completed_at: None,
                outcome: None,
                error: None,
                evidence: Vec::new(),
            };
            save_runs(&loc, &[run]).unwrap();
        }

        let result = reclaim_stale(&loc, &RunLimits::default()).unwrap();
        assert_eq!(result.reclaimed_count, 1);
        assert_eq!(result.blocked_count, 0);
        assert_eq!(result.details[0].new_card_status, "todo");

        let snap = ops::list(&loc).unwrap();
        assert_eq!(snap.cards[0].status, TaskCardStatus::Todo);
    }

    #[test]
    fn reclaim_blocks_after_max_reclaims() {
        let dir = tempdir().unwrap();
        let loc = thread_loc(dir.path(), "reclaim-block-test");

        let snap = ops::add(&loc, "troublesome task", CardPatch::default()).unwrap();
        let card_id = snap.cards[0].id.clone();
        ops::update_status(&loc, &card_id, TaskCardStatus::InProgress).unwrap();

        // Seed prior reclaimed runs (3 = at the limit)
        {
            let lock = run_lock(&loc);
            let _guard = lock.lock();
            let old = (Utc::now() - chrono::Duration::seconds(600)).to_rfc3339();
            let mut runs = Vec::new();
            for i in 0..3 {
                runs.push(TaskRun {
                    run_id: format!("prior-{i}"),
                    card_id: card_id.clone(),
                    claimed_by: "test".into(),
                    claim_token: format!("t{i}"),
                    started_at: old.clone(),
                    last_heartbeat_at: old.clone(),
                    completed_at: Some(old.clone()),
                    outcome: Some(RunOutcome::Reclaimed),
                    error: Some("stale".into()),
                    evidence: Vec::new(),
                });
            }
            // Active stale run
            runs.push(TaskRun {
                run_id: "current-stale".into(),
                card_id: card_id.clone(),
                claimed_by: "test".into(),
                claim_token: "tc".into(),
                started_at: old.clone(),
                last_heartbeat_at: old,
                completed_at: None,
                outcome: None,
                error: None,
                evidence: Vec::new(),
            });
            save_runs(&loc, &runs).unwrap();
        }

        let result = reclaim_stale(&loc, &RunLimits::default()).unwrap();
        assert_eq!(result.reclaimed_count, 0);
        assert_eq!(result.blocked_count, 1);
        assert_eq!(result.details[0].new_card_status, "blocked");

        let snap = ops::list(&loc).unwrap();
        assert_eq!(snap.cards[0].status, TaskCardStatus::Blocked);
        assert!(snap.cards[0]
            .blocker
            .as_deref()
            .unwrap_or_default()
            .contains("Reclaimed"));
    }

    #[test]
    fn reclaim_skips_healthy_runs() {
        let dir = tempdir().unwrap();
        let loc = thread_loc(dir.path(), "healthy-test");

        let snap = ops::add(&loc, "healthy task", CardPatch::default()).unwrap();
        let card_id = snap.cards[0].id.clone();
        ops::update_status(&loc, &card_id, TaskCardStatus::InProgress).unwrap();

        create_run(&loc, "healthy-run", &card_id, "default").unwrap();

        let result = reclaim_stale(&loc, &RunLimits::default()).unwrap();
        assert_eq!(result.reclaimed_count, 0);
        assert_eq!(result.blocked_count, 0);
    }

    #[test]
    fn scratch_location_returns_empty_runs() {
        let runs = list_runs(&BoardLocation::Scratch, None).unwrap();
        assert!(runs.is_empty());
    }
}
