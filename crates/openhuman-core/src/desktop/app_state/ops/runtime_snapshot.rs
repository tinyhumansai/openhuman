//! Building the `runtime` half of `app_state_snapshot`: local-AI and service
//! status, fanned out concurrently and cached briefly so overlapping polls
//! collapse onto one rebuild instead of each re-running the sub-op fan-out.

use super::types::RuntimeSnapshot;
use super::LOG_PREFIX;
use crate::config::Config;
use crate::platform::service::ServiceState;
use crate::platform::service::ServiceStatus;
use log::{debug, warn};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Runtime-status widgets (local AI / autocomplete / service) tolerate ~10s of
/// staleness. A short TTL (was 2s < the ~2.4s build
/// time) meant the cache was stale before it was even written, so the frontend's
/// ~4s `app_state_snapshot` poll never hit the fast path and every poll re-ran
/// the full 4-way fan-out (issue #4249 profiling: this, combined with the lack
/// of a single-flight gate, pegged ~2 cores and starved the shared tokio runtime
/// the agent harness runs on — the agent's turns stalled 50-100s between model
/// calls even though inference itself was idle).
pub(super) const RUNTIME_SNAPSHOT_TTL: Duration = Duration::from_secs(10);
pub(super) const RUNTIME_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(10);
pub(super) const SNAPSHOT_SUB_OP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub(super) struct CachedRuntimeSnapshot {
    pub(super) snapshot: RuntimeSnapshot,
    pub(super) fetched_at: Instant,
    /// Config identity (`workspace_dir`) the snapshot was built for. The cache
    /// holds one entry process-wide, so a snapshot built for one config must
    /// never be served to another — otherwise a different user/workspace (or an
    /// E2E test with an injected service mock) reads a stale, foreign runtime.
    pub(super) config_key: PathBuf,
}

pub(super) static RUNTIME_SNAPSHOT_CACHE: Lazy<Mutex<Option<CachedRuntimeSnapshot>>> =
    Lazy::new(|| Mutex::new(None));
/// Single-flight gate for the runtime-snapshot rebuild. Concurrent callers whose
/// cache read missed serialize here so only ONE runs the expensive sub-op
/// fan-out; the rest wait, then re-read the cache the winner populated (see the
/// double-check in `build_runtime_snapshot`). This is an async mutex because the
/// guard is held across `.await` points (the sub-op `join`). Without it, every
/// overlapping `app_state_snapshot` poll launched its own build — the rebuild
/// stampede described on `RUNTIME_SNAPSHOT_TTL`.
pub(super) static RUNTIME_SNAPSHOT_REBUILD: Lazy<tokio::sync::Mutex<()>> =
    Lazy::new(|| tokio::sync::Mutex::new(()));

/// Return the cached runtime snapshot when it is still within
/// `RUNTIME_SNAPSHOT_TTL`, else `None`. Kept as a small helper so both the
/// fast-path read and the post-lock double-check share identical freshness logic.
/// A service-status mock is injected via `OPENHUMAN_SERVICE_MOCK` (test-only env
/// hook that production `service` status already honors). While it is active the
/// runtime snapshot must never be served from — or written to — the process-
/// global cache: the mock's state changes between calls, so caching it would
/// both mask the freshly-injected value and poison later (non-mocked) reads.
pub(super) fn service_status_mock_active() -> bool {
    std::env::var_os("OPENHUMAN_SERVICE_MOCK").is_some()
}

pub(super) fn fresh_cached_runtime_snapshot(
    config: &Config,
    req_id: u64,
) -> Option<RuntimeSnapshot> {
    if service_status_mock_active() {
        return None;
    }
    let cache = RUNTIME_SNAPSHOT_CACHE.lock();
    let entry = cache.as_ref()?;
    // A snapshot built for a different config identity is a miss: rebuild against
    // this config rather than serve another workspace's runtime.
    if entry.config_key != config.workspace_dir {
        return None;
    }
    let age = entry.fetched_at.elapsed();
    if age < RUNTIME_SNAPSHOT_TTL {
        debug!(
            "{LOG_PREFIX} build_runtime_snapshot: returning cached snapshot req_id={req_id} age_ms={}",
            age.as_millis()
        );
        Some(entry.snapshot.clone())
    } else {
        None
    }
}

pub(super) async fn build_runtime_snapshot(config: &Config, req_id: u64) -> RuntimeSnapshot {
    // Fast path: a fresh cached snapshot serves every poller without touching the
    // sub-op fan-out.
    if let Some(snapshot) = fresh_cached_runtime_snapshot(config, req_id) {
        return snapshot;
    }

    // Cache miss: single-flight the rebuild so only one caller runs the expensive
    // fan-out. Waiters re-check the cache the winner just populated (this
    // double-check) and return it instead of launching a duplicate build —
    // collapsing an N-way stampede into one build per TTL window.
    let _rebuild_guard = RUNTIME_SNAPSHOT_REBUILD.lock().await;
    if let Some(snapshot) = fresh_cached_runtime_snapshot(config, req_id) {
        debug!(
            "{LOG_PREFIX} build_runtime_snapshot: coalesced onto concurrent rebuild req_id={req_id}"
        );
        return snapshot;
    }

    let config_for_local_ai = config.clone();
    let config_for_service = config.clone();

    let t0 = Instant::now();

    let (local_ai, service) = tokio::join!(
        async {
            let t = Instant::now();
            let status = match tokio::time::timeout(
                SNAPSHOT_SUB_OP_TIMEOUT,
                crate::inference::ops::inference_status(&config_for_local_ai),
            )
            .await
            {
                Ok(Ok(outcome)) => outcome.value,
                Ok(Err(error)) => {
                    warn!("{LOG_PREFIX} local_ai status failed during snapshot: {error}");
                    crate::inference::disabled_local_ai_status(&config_for_local_ai)
                }
                Err(_) => {
                    warn!(
                        "{LOG_PREFIX} local_ai timed out after {}s; using degraded sub-snapshot req_id={}",
                        SNAPSHOT_SUB_OP_TIMEOUT.as_secs(),
                        req_id,
                    );
                    crate::inference::disabled_local_ai_status(&config_for_local_ai)
                }
            };
            (status, t.elapsed().as_millis())
        },
        async {
            let t = Instant::now();
            let status = tokio::task::spawn_blocking(move || {
                crate::platform::service::status(&config_for_service)
            })
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("service status task panicked")));
            let status = match status {
                Ok(s) => s,
                Err(error) => {
                    let message = error.to_string();
                    warn!("{LOG_PREFIX} service status failed during snapshot: {message}");
                    ServiceStatus {
                        state: ServiceState::Unknown(message.clone()),
                        unit_path: None,
                        label: "OpenHuman".to_string(),
                        details: Some(message),
                    }
                }
            };
            (status, t.elapsed().as_millis())
        }
    );

    let total_ms = t0.elapsed().as_millis();
    debug!(
        "{LOG_PREFIX} build_runtime_snapshot timings req_id={} local_ai_ms={} service_ms={} total_ms={}",
        req_id,
        local_ai.1, service.1,
        total_ms,
    );

    let snapshot = RuntimeSnapshot {
        local_ai: local_ai.0,
        service: service.0,
    };

    // Don't cache a snapshot built under an injected service mock (see
    // `service_status_mock_active`) — it would poison later non-mocked reads.
    if !service_status_mock_active() {
        *RUNTIME_SNAPSHOT_CACHE.lock() = Some(CachedRuntimeSnapshot {
            snapshot: snapshot.clone(),
            fetched_at: Instant::now(),
            config_key: config.workspace_dir.clone(),
        });
    }

    snapshot
}

pub(super) fn degraded_runtime_snapshot(config: &Config) -> RuntimeSnapshot {
    RuntimeSnapshot {
        local_ai: crate::inference::disabled_local_ai_status(config),
        service: ServiceStatus {
            state: ServiceState::Unknown("snapshot timed out".to_string()),
            unit_path: None,
            label: "OpenHuman".to_string(),
            details: Some("runtime snapshot timed out".to_string()),
        },
    }
}
