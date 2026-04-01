//! Per-session serialised lane queue.
//!
//! All incoming tasks are serialised per-session to prevent race conditions when
//! writing to files, memory, or other shared resources. Cross-session requests
//! run concurrently.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

/// A queue that serialises work within a session while allowing parallelism
/// across sessions.
///
/// Each session ID maps to a `Semaphore(1)`. Acquiring the permit blocks
/// subsequent requests for the *same* session until the permit is released.
pub struct SessionQueue {
    lanes: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl SessionQueue {
    pub fn new() -> Self {
        Self {
            lanes: Mutex::new(HashMap::new()),
        }
    }

    /// Acquire the lane for `session_id`.
    ///
    /// Returns an `OwnedSemaphorePermit` that the caller must hold for the
    /// duration of the request. Subsequent requests on the same session will
    /// block until this permit is dropped.
    pub async fn acquire(&self, session_id: &str) -> OwnedSemaphorePermit {
        let sem = {
            let mut map = self.lanes.lock().await;
            let is_new = !map.contains_key(session_id);
            let sem = map
                .entry(session_id.to_string())
                .or_insert_with(|| Arc::new(Semaphore::new(1)))
                .clone();
            if is_new {
                tracing::trace!("[session-queue] created lane for session={session_id}");
            }
            tracing::trace!(
                "[session-queue] acquiring lane session={session_id}, permits={}",
                sem.available_permits()
            );
            sem
        };
        let permit = sem.acquire_owned().await.expect("session semaphore closed");
        tracing::trace!("[session-queue] acquired lane for session={session_id}");
        permit
    }

    /// Remove stale session lanes that have no waiters.
    /// Call periodically or after sessions end to prevent unbounded growth.
    pub async fn gc(&self) {
        let mut map = self.lanes.lock().await;
        let before = map.len();
        map.retain(|id, sem| {
            let keep = sem.available_permits() < 1 || Arc::strong_count(sem) > 1;
            if !keep {
                tracing::trace!("[session-queue] pruning idle lane session={id}");
            }
            keep
        });
        let removed = before - map.len();
        if removed > 0 {
            tracing::debug!(
                "[session-queue] gc removed {removed} idle lane(s), {} remaining",
                map.len()
            );
        }
    }

    /// Number of tracked session lanes (for diagnostics).
    pub async fn lane_count(&self) -> usize {
        self.lanes.lock().await.len()
    }
}

impl Default for SessionQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn serialises_within_same_session() {
        let queue = Arc::new(SessionQueue::new());
        let counter = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..5 {
            let q = queue.clone();
            let c = counter.clone();
            handles.push(tokio::spawn(async move {
                let _permit = q.acquire("session-1").await;
                // If serialised, at most 1 task holds the permit at a time.
                let prev = c.fetch_add(1, Ordering::SeqCst);
                // While we hold the permit, sleep briefly.
                sleep(Duration::from_millis(10)).await;
                let current = c.load(Ordering::SeqCst);
                // Nobody else should have incremented while we held the permit.
                assert_eq!(current, prev + 1);
            }));
        }

        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(counter.load(Ordering::SeqCst), 5);
    }

    #[tokio::test]
    async fn parallel_across_sessions() {
        let queue = Arc::new(SessionQueue::new());
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for i in 0..4 {
            let q = queue.clone();
            let a = active.clone();
            let m = max_active.clone();
            let session = format!("session-{i}");
            handles.push(tokio::spawn(async move {
                let _permit = q.acquire(&session).await;
                let current = a.fetch_add(1, Ordering::SeqCst) + 1;
                m.fetch_max(current, Ordering::SeqCst);
                sleep(Duration::from_millis(50)).await;
                a.fetch_sub(1, Ordering::SeqCst);
            }));
        }

        for h in handles {
            h.await.unwrap();
        }
        // Multiple sessions should have run concurrently.
        assert!(max_active.load(Ordering::SeqCst) > 1);
    }

    #[tokio::test]
    async fn gc_removes_idle_lanes() {
        let queue = SessionQueue::new();
        {
            let _permit = queue.acquire("temp-session").await;
        }
        // Permit dropped, lane is idle.
        queue.gc().await;
        assert_eq!(queue.lane_count().await, 0);
    }
}
