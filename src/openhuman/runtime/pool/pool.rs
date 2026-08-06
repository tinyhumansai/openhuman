//! The bounded per-language worker pool and its process-global registry.
//!
//! One [`LangPool`] owns up to `max_workers` warm [`PoolWorker`]s for a single
//! language. Concurrency is bounded by a semaphore; submissions beyond the pool
//! size **queue** on that semaphore (the intended backpressure), and
//! submissions beyond `max_workers + max_queue_depth` in flight are rejected so
//! memory can't grow without bound. Idle workers are reaped after a TTL; busy
//! workers are recycled after N jobs.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::sync::{Mutex, Semaphore};

use super::protocol::{PoolJobRequest, PoolJobResponse};
use super::types::{PoolExecOutcome, PoolLang, PoolSettings};
use super::worker::{PoolWorker, WorkerLaunch};

/// Why a pooled run failed, classified so callers know whether a retry or a
/// legacy per-call-spawn fallback is safe.
#[derive(Debug)]
pub enum PoolRunError {
    /// In-flight work exceeded `max_workers + max_queue_depth`; the pool shed
    /// load rather than buffering unbounded. Callers must **not** fall back to a
    /// per-call spawn — that reintroduces the very RSS the pool caps (#5106) —
    /// but surface a busy error or retry later.
    Saturated,
    /// Failure before the job reached a worker (serialise / spawn / write). The
    /// job never ran, so a retry or a legacy-spawn fallback is safe.
    PreDispatch(anyhow::Error),
    /// Failure after the job was dispatched (it may have executed). Terminal —
    /// the caller must not re-run it (would duplicate side effects).
    PostDispatch(anyhow::Error),
}

impl std::fmt::Display for PoolRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PoolRunError::Saturated => write!(f, "runtime pool at capacity"),
            PoolRunError::PreDispatch(e) => write!(f, "pre-dispatch pool failure: {e:#}"),
            PoolRunError::PostDispatch(e) => write!(f, "post-dispatch pool failure: {e:#}"),
        }
    }
}

/// Extra grace added to a job's soft deadline before the Rust side treats the
/// worker as wedged and kills it. The worker should always self-abort first.
const HARD_TIMEOUT_GRACE: Duration = Duration::from_secs(10);

/// Lower bound on how often the idle reaper wakes, so a tiny TTL doesn't busy-loop.
const MIN_REAP_INTERVAL: Duration = Duration::from_secs(5);

/// Lightweight, cloneable snapshot of a pool's counters (for status/tests).
#[derive(Debug, Clone, Default)]
pub struct PoolStats {
    pub jobs_total: u64,
    pub worker_spawns: u64,
    pub rejected_saturated: u64,
    pub idle_workers: usize,
    pub max_workers: usize,
}

pub struct LangPool {
    launch: WorkerLaunch,
    settings: PoolSettings,
    /// Permits == max_workers. Acquiring one is the queue/backpressure gate.
    permits: Arc<Semaphore>,
    /// Warm, idle workers available for reuse.
    idle: Mutex<Vec<PoolWorker>>,
    /// Jobs currently in flight or waiting for a permit (for saturation guard).
    inflight: AtomicUsize,
    job_seq: AtomicU64,
    jobs_total: AtomicU64,
    worker_spawns: AtomicU64,
    rejected_saturated: AtomicU64,
}

impl LangPool {
    /// Build a pool and start its idle reaper. The reaper holds only a `Weak`
    /// ref, so the pool is still dropped normally when the registry evicts it.
    pub fn start(launch: WorkerLaunch, settings: PoolSettings) -> Arc<Self> {
        let permits = Arc::new(Semaphore::new(settings.max_workers));
        let pool = Arc::new(Self {
            launch,
            settings,
            permits,
            idle: Mutex::new(Vec::new()),
            inflight: AtomicUsize::new(0),
            job_seq: AtomicU64::new(0),
            jobs_total: AtomicU64::new(0),
            worker_spawns: AtomicU64::new(0),
            rejected_saturated: AtomicU64::new(0),
        });
        if let Some(ttl) = pool.settings.idle_ttl {
            spawn_reaper(Arc::downgrade(&pool), ttl);
        }
        pool
    }

    pub fn lang(&self) -> PoolLang {
        self.launch.lang
    }

    pub async fn stats(&self) -> PoolStats {
        PoolStats {
            jobs_total: self.jobs_total.load(Ordering::Relaxed),
            worker_spawns: self.worker_spawns.load(Ordering::Relaxed),
            rejected_saturated: self.rejected_saturated.load(Ordering::Relaxed),
            idle_workers: self.idle.lock().await.len(),
            max_workers: self.settings.max_workers,
        }
    }

    /// Run one inline job, blocking (asynchronously) until a worker is free.
    pub async fn run_inline(
        &self,
        code: String,
        cwd: Option<String>,
        timeout: Option<Duration>,
    ) -> Result<PoolExecOutcome, PoolRunError> {
        // Saturation guard: bound total in-flight (running + queued) work so a
        // stampede queues up to a point, then sheds load instead of buffering
        // unbounded. Capacity = worker slots + allowed queue depth.
        let capacity = self.settings.max_workers + self.settings.max_queue_depth;
        let inflight_now = self.inflight.fetch_add(1, Ordering::AcqRel) + 1;
        if inflight_now > capacity {
            self.inflight.fetch_sub(1, Ordering::AcqRel);
            self.rejected_saturated.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                lang = self.launch.lang.id(),
                inflight = inflight_now,
                capacity,
                "[runtime_pool] saturated; shedding load (no spawn fallback)"
            );
            return Err(PoolRunError::Saturated);
        }
        // Ensure the in-flight counter is released on every exit path below.
        let _inflight_guard = InflightGuard(&self.inflight);

        let wait_start = Instant::now();
        let _permit = self
            .permits
            .acquire()
            .await
            .expect("runtime pool semaphore never closed");
        let queue_wait = wait_start.elapsed();

        let id = self.job_seq.fetch_add(1, Ordering::Relaxed).to_string();
        let req = PoolJobRequest {
            id,
            kind: "inline".to_string(),
            code: Some(code),
            cwd,
            timeout_ms: timeout.map(|t| t.as_millis() as u64),
        };
        let hard_timeout = timeout.map(|t| t + HARD_TIMEOUT_GRACE);

        let job_start = Instant::now();
        let (response, worker) = self.submit_with_retry(&req, hard_timeout).await?;
        let elapsed = job_start.elapsed();

        self.jobs_total.fetch_add(1, Ordering::Relaxed);

        // Recycle after N jobs, otherwise return the warm worker to the pool.
        if worker.should_recycle(self.settings.recycle_after_jobs) {
            tracing::debug!(
                lang = self.launch.lang.id(),
                jobs = worker.jobs_done(),
                "[runtime_pool] recycling worker after job budget"
            );
            worker.shutdown();
        } else {
            self.idle.lock().await.push(worker);
        }

        if let Some(err) = response.error {
            // The worker replied with a harness-level error: the job was
            // dispatched (and may have run), so this is terminal.
            return Err(PoolRunError::PostDispatch(anyhow::anyhow!(
                "{} worker error: {err}",
                self.launch.lang.id()
            )));
        }
        Ok(PoolExecOutcome {
            stdout: response.stdout,
            stderr: response.stderr,
            exit_code: response.exit_code,
            timed_out: response.timed_out,
            elapsed,
            queue_wait,
        })
    }

    /// Submit on a warm-or-fresh worker. A **pre-dispatch** failure (e.g. a
    /// reused idle worker that died on write) respawns once — the job never ran,
    /// so that is safe. A **post-dispatch** failure is terminal: the job may have
    /// executed, so it is never re-run (no duplicate side effects). Returns the
    /// surviving worker so the caller can recycle or re-pool it.
    async fn submit_with_retry(
        &self,
        req: &PoolJobRequest,
        hard_timeout: Option<Duration>,
    ) -> Result<(PoolJobResponse, PoolWorker), PoolRunError> {
        let mut worker = self
            .take_or_spawn()
            .await
            .map_err(PoolRunError::PreDispatch)?;
        match worker.submit(req, hard_timeout).await {
            Ok(resp) => Ok((resp, worker)),
            Err(e) if !e.dispatched => {
                tracing::warn!(
                    lang = self.launch.lang.id(),
                    "[runtime_pool] pre-dispatch submit failure ({e}); respawning once"
                );
                worker.shutdown();
                let mut fresh = self
                    .spawn_worker()
                    .await
                    .map_err(PoolRunError::PreDispatch)?;
                match fresh.submit(req, hard_timeout).await {
                    Ok(resp) => Ok((resp, fresh)),
                    Err(e2) if !e2.dispatched => Err(PoolRunError::PreDispatch(e2.err)),
                    Err(e2) => Err(PoolRunError::PostDispatch(e2.err)),
                }
            }
            Err(e) => {
                tracing::warn!(
                    lang = self.launch.lang.id(),
                    "[runtime_pool] post-dispatch submit failure ({e}); terminal, not retrying"
                );
                worker.shutdown();
                Err(PoolRunError::PostDispatch(e.err))
            }
        }
    }

    /// Pop a still-fresh idle worker, or spawn a new one.
    async fn take_or_spawn(&self) -> Result<PoolWorker> {
        {
            let mut idle = self.idle.lock().await;
            while let Some(worker) = idle.pop() {
                if let Some(ttl) = self.settings.idle_ttl {
                    if worker.idle_expired(ttl) {
                        worker.shutdown();
                        continue;
                    }
                }
                return Ok(worker);
            }
        }
        self.spawn_worker().await
    }

    async fn spawn_worker(&self) -> Result<PoolWorker> {
        self.worker_spawns.fetch_add(1, Ordering::Relaxed);
        PoolWorker::spawn(&self.launch).await
    }

    /// Drop workers idle beyond the TTL. Called by the background reaper.
    async fn reap_idle(&self) {
        let Some(ttl) = self.settings.idle_ttl else {
            return;
        };
        let mut idle = self.idle.lock().await;
        let before = idle.len();
        let mut kept = Vec::with_capacity(before);
        for worker in idle.drain(..) {
            if worker.idle_expired(ttl) {
                worker.shutdown();
            } else {
                kept.push(worker);
            }
        }
        let reaped = before - kept.len();
        *idle = kept;
        if reaped > 0 {
            tracing::debug!(
                lang = self.launch.lang.id(),
                reaped,
                "[runtime_pool] idle reaper retired workers"
            );
        }
    }
}

/// Decrements the in-flight counter on drop so every early return is covered.
struct InflightGuard<'a>(&'a AtomicUsize);
impl Drop for InflightGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn spawn_reaper(pool: Weak<LangPool>, ttl: Duration) {
    let interval = ttl.max(MIN_REAP_INTERVAL);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            match pool.upgrade() {
                Some(pool) => pool.reap_idle().await,
                None => break, // pool dropped — stop reaping
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Process-global registry
// ---------------------------------------------------------------------------

struct CachedPool {
    key: String,
    pool: Arc<LangPool>,
}

static REGISTRY: OnceLock<Mutex<HashMap<PoolLangKey, CachedPool>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum PoolLangKey {
    Node,
    Python,
}

impl From<PoolLang> for PoolLangKey {
    fn from(lang: PoolLang) -> Self {
        match lang {
            PoolLang::Node => PoolLangKey::Node,
            PoolLang::Python => PoolLangKey::Python,
        }
    }
}

fn registry() -> &'static Mutex<HashMap<PoolLangKey, CachedPool>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Fingerprint that decides whether a cached pool can be reused. Any change to
/// the interpreter path, args, or tuning rebuilds the pool.
fn launch_key(launch: &WorkerLaunch, settings: &PoolSettings) -> String {
    format!(
        "{}|{}|{:?}|isolated={}|w={}|ttl={:?}|recycle={}|q={}",
        launch.lang.id(),
        launch.bin.display(),
        launch.args,
        launch.isolated_protocol,
        settings.max_workers,
        settings.idle_ttl,
        settings.recycle_after_jobs,
        settings.max_queue_depth,
    )
}

/// Get (or build) the process-global pool for a language, keyed by its launch
/// fingerprint. A config or interpreter change transparently rebuilds it.
pub async fn ensure_pool(launch: WorkerLaunch, settings: PoolSettings) -> Arc<LangPool> {
    let key = launch_key(&launch, &settings);
    let lang_key = PoolLangKey::from(launch.lang);
    let mut reg = registry().lock().await;
    if let Some(cached) = reg.get(&lang_key) {
        if cached.key == key {
            return cached.pool.clone();
        }
        tracing::info!(
            lang = launch.lang.id(),
            "[runtime_pool] launch spec changed; rebuilding pool"
        );
    }
    let pool = LangPool::start(launch, settings);
    reg.insert(
        lang_key,
        CachedPool {
            key,
            pool: pool.clone(),
        },
    );
    pool
}

/// Snapshot every live pool's stats (for a status surface / debugging).
pub async fn all_stats() -> Vec<(PoolLang, PoolStats)> {
    let reg = registry().lock().await;
    let mut out = Vec::new();
    for cached in reg.values() {
        out.push((cached.pool.lang(), cached.pool.stats().await));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_key_changes_with_tuning() {
        let launch = WorkerLaunch {
            lang: PoolLang::Node,
            bin: "/usr/bin/node".into(),
            args: vec!["worker.js".into()],
            env: vec![],
            isolated_protocol: true,
        };
        let a = PoolSettings {
            max_workers: 2,
            idle_ttl: Some(Duration::from_secs(60)),
            recycle_after_jobs: 100,
            max_queue_depth: 256,
        };
        let mut b = a.clone();
        b.max_workers = 4;
        assert_ne!(launch_key(&launch, &a), launch_key(&launch, &b));
        assert_eq!(launch_key(&launch, &a), launch_key(&launch, &a));
    }

    #[test]
    fn pool_run_error_display_is_classified() {
        // The three arms drive distinct caller behaviour (retry / fall back /
        // give up), so their rendered messages must stay distinguishable.
        assert_eq!(
            PoolRunError::Saturated.to_string(),
            "runtime pool at capacity"
        );
        let pre = PoolRunError::PreDispatch(anyhow::anyhow!("spawn failed")).to_string();
        assert!(pre.starts_with("pre-dispatch pool failure:"), "got {pre}");
        assert!(pre.contains("spawn failed"));
        let post = PoolRunError::PostDispatch(anyhow::anyhow!("read wedged")).to_string();
        assert!(
            post.starts_with("post-dispatch pool failure:"),
            "got {post}"
        );
        assert!(post.contains("read wedged"));
    }
}
