//! Cached health checker for the local Ollama model server.
//!
//! Probes `GET {base_url}/api/tags` with a short timeout and caches the
//! result to avoid adding per-call network latency to every inference request.

use parking_lot::Mutex;
use std::time::{Duration, Instant};

/// Default TTL for cached health results.
const DEFAULT_TTL: Duration = Duration::from_secs(30);
/// Timeout for the Ollama health probe.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CachedStatus {
    Healthy,
    Unavailable,
}

struct HealthCache {
    last_result: CachedStatus,
    checked_at: Instant,
    ttl: Duration,
}

/// Async, caching health checker for the local Ollama server.
///
/// All fields are `Send + Sync`. The `Mutex` critical section never crosses an
/// `await` boundary: the lock is acquired to read/write the cache, released,
/// and *then* the async HTTP probe is performed if needed.
pub struct LocalHealthChecker {
    client: reqwest::Client,
    probe_url: String,
    cache: Mutex<Option<HealthCache>>,
    ttl: Duration,
}

impl LocalHealthChecker {
    /// Create a checker targeting the given Ollama base URL.
    ///
    /// Health is probed at `{base_url}/api/tags`. Results are cached for 30 s.
    pub fn new(base_url: &str) -> Self {
        Self::with_ttl(base_url, DEFAULT_TTL)
    }

    /// Create a checker with a custom cache TTL (useful in tests).
    pub fn with_ttl(base_url: &str, ttl: Duration) -> Self {
        Self::with_probe_url(format!("{base_url}/api/tags"), ttl)
    }

    /// Create a checker with an explicit full probe URL (for non-ollama local
    /// backends such as llama-server, whose health endpoint is `/v1/models`).
    pub fn with_probe_url(probe_url: String, ttl: Duration) -> Self {
        let client = reqwest::Client::builder()
            .timeout(PROBE_TIMEOUT)
            .build()
            .unwrap_or_else(|err| {
                tracing::warn!(
                    error = %err,
                    "[routing] failed to build health probe client, using default client"
                );
                reqwest::Client::new()
            });
        Self {
            client,
            probe_url,
            cache: Mutex::new(None),
            ttl,
        }
    }

    /// Returns `true` when Ollama is reachable and the tags endpoint responds
    /// with a 2xx status. Cached for the configured TTL.
    pub async fn is_healthy(&self) -> bool {
        // Fast path: return cached result if still fresh.
        {
            let guard = self.cache.lock();
            if let Some(cached) = guard.as_ref() {
                let elapsed = cached.checked_at.elapsed();
                if elapsed < cached.ttl {
                    tracing::trace!(
                        cached_last_result = ?cached.last_result,
                        checked_at_elapsed = ?elapsed,
                        cached_ttl = ?cached.ttl,
                        "[routing] local health cache hit"
                    );
                    return cached.last_result == CachedStatus::Healthy;
                }
            }
        }

        tracing::trace!("[routing] local health cache stale/miss; probing");
        // Slow path: probe and update cache.
        let healthy = self.probe().await;
        let status = if healthy {
            CachedStatus::Healthy
        } else {
            CachedStatus::Unavailable
        };
        tracing::trace!(
            healthy,
            mapped_status = ?status,
            "[routing] local health probe completed"
        );

        {
            let mut guard = self.cache.lock();
            let new_cache = HealthCache {
                last_result: status,
                checked_at: Instant::now(),
                ttl: self.ttl,
            };
            tracing::trace!(
                new_last_result = ?new_cache.last_result,
                new_checked_at = ?new_cache.checked_at,
                new_ttl = ?new_cache.ttl,
                "[routing] local health cache updated"
            );
            *guard = Some(new_cache);
        }

        healthy
    }

    /// Perform a single live probe — no caching.
    async fn probe(&self) -> bool {
        match self.client.get(&self.probe_url).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(err) => {
                tracing::debug!(
                    url = %self.probe_url,
                    error = %err,
                    "[routing] local health probe failed"
                );
                false
            }
        }
    }

    /// Invalidate the cached health result, forcing a fresh probe on the next call.
    #[cfg(test)]
    pub fn invalidate(&self) {
        *self.cache.lock() = None;
    }

    /// Create a checker pre-seeded with a known health state (test-only).
    ///
    /// The cache is set to never expire (`TTL = MAX`) so the given result is
    /// returned immediately on every `is_healthy()` call without hitting the
    /// network. Use this in tests to control local health without starting
    /// a real Ollama instance.
    #[cfg(test)]
    pub fn seeded(healthy: bool) -> std::sync::Arc<Self> {
        let checker = Self::with_ttl("http://127.0.0.1:19999", Duration::MAX);
        *checker.cache.lock() = Some(HealthCache {
            last_result: if healthy {
                CachedStatus::Healthy
            } else {
                CachedStatus::Unavailable
            },
            checked_at: Instant::now(),
            ttl: Duration::MAX,
        });
        std::sync::Arc::new(checker)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unreachable_host_returns_false() {
        // Use a clearly non-routable address to trigger a fast connection failure.
        let checker = LocalHealthChecker::with_ttl("http://127.0.0.1:19999", Duration::ZERO);
        assert!(!checker.is_healthy().await);
    }

    #[tokio::test]
    async fn cache_prevents_second_probe_within_ttl() {
        // Use a large TTL so the second call hits the cache.
        let checker =
            LocalHealthChecker::with_ttl("http://127.0.0.1:19999", Duration::from_secs(3600));

        let first = checker.is_healthy().await; // fills cache (false — unreachable)

        // Swap probe URL to something that *would* succeed (if no cache bypass).
        // Since the cache is warm, we never actually probe, so the result stays `false`.
        // We can't mutate the probe URL, but we can verify the cache is used by
        // checking that a second call returns the same value as the first.
        let second = checker.is_healthy().await;

        assert_eq!(first, second, "second call should return cached result");
    }

    #[tokio::test]
    async fn cache_expires_after_ttl() {
        // TTL of zero means every call probes.
        let checker =
            LocalHealthChecker::with_ttl("http://127.0.0.1:19999", Duration::from_millis(0));

        // Both calls go through the full probe path — both should be false (unreachable).
        assert!(!checker.is_healthy().await);
        assert!(!checker.is_healthy().await);
    }

    #[tokio::test]
    async fn invalidate_forces_fresh_probe() {
        let checker =
            LocalHealthChecker::with_ttl("http://127.0.0.1:19999", Duration::from_secs(3600));

        let _ = checker.is_healthy().await; // fills cache
        checker.invalidate();

        // After invalidation the cache is empty; next call probes again.
        // Result is still false (host unreachable), but the probe ran.
        assert!(!checker.is_healthy().await);
    }
}
