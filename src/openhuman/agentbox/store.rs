//! In-memory job store for AgentBox jobs.
//!
//! Tracks each `POST /run` invocation by uuid. Terminal jobs (`completed` /
//! `failed`) are retained for the configured window then evicted by
//! [`JobStore::sweep_now`]. Running and pending jobs are never evicted.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

use super::types::{JobRecord, JobStatus, JobView, RunResult};

/// Thread-safe in-memory job store with terminal-job retention sweeping.
#[derive(Clone)]
pub struct JobStore {
    inner: Arc<RwLock<HashMap<String, JobRecord>>>,
    retention: Duration,
}

impl JobStore {
    pub fn new(retention: Duration) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            retention,
        }
    }

    pub fn insert_pending(&self) -> String {
        let id = Uuid::new_v4().to_string();
        self.inner
            .write()
            .insert(id.clone(), JobRecord::new_pending());
        id
    }

    pub fn get(&self, id: &str) -> Option<JobView> {
        self.inner.read().get(id).map(|r| r.view())
    }

    pub fn mark_running(&self, id: &str) {
        if let Some(rec) = self.inner.write().get_mut(id) {
            rec.status = JobStatus::Running;
        }
    }

    pub fn mark_completed(&self, id: &str, result: RunResult) {
        if let Some(rec) = self.inner.write().get_mut(id) {
            rec.status = JobStatus::Completed;
            rec.result = Some(result);
            rec.error = None;
            rec.terminal_at = Some(Instant::now());
        }
    }

    pub fn mark_failed(&self, id: &str, error: String) {
        if let Some(rec) = self.inner.write().get_mut(id) {
            rec.status = JobStatus::Failed;
            rec.result = None;
            rec.error = Some(error);
            rec.terminal_at = Some(Instant::now());
        }
    }

    /// Evict terminal jobs whose `terminal_at` is older than the retention
    /// window. Returns the number of jobs removed.
    pub fn sweep_now(&self) -> usize {
        let now = Instant::now();
        let retention = self.retention;
        let mut guard = self.inner.write();
        let before = guard.len();
        guard.retain(|_, rec| match rec.terminal_at {
            Some(t) => now.duration_since(t) < retention,
            None => true,
        });
        before - guard.len()
    }
}
