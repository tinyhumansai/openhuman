//! Client-side request-rate limiting for cloud embedding backends.
//!
//! Cloud embedding backends (the OpenHuman backend / Voyage, OpenAI, and any
//! OpenAI-compatible `custom:` endpoint) cap requests at a fixed rate per
//! account — ~60/min by default. Every [`super::openai::OpenAiEmbedding::embed`]
//! call is exactly one HTTP POST, and memory-tree ingest fans out one call per
//! chunk across several job workers, so without throttling we routinely trip
//! the backend limiter and absorb `429`s (which `openai.rs` currently
//! downgrades to a warning breadcrumb). This module spends the budget
//! *proactively* so requests stay under the quota instead of reacting to 429s.
//!
//! ## Why a process-global registry keyed by endpoint
//!
//! The quota is account-wide, not per-instance — and `OpenAiEmbedding`
//! instances are ephemeral (the cloud provider builds a fresh one on every
//! `embed` call, and the embedder is constructed from several independent
//! sites: the memory factory, `default_embedding_provider`, and the
//! memory-tree score path). A per-instance limiter would therefore reset
//! constantly and enforce nothing. Instead the buckets live in a
//! process-global registry keyed by the resolved base URL, so all ephemeral
//! instances pointing at the same backend share one budget while distinct
//! backends get independent budgets.
//!
//! Loopback endpoints (`localhost` / `127.0.0.1` / `::1`) are exempt: a local
//! LocalAI- or Ollama-compatible `custom:` server is not the remote quota this
//! guards, and capping it to 60/min would needlessly throttle local
//! throughput.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

/// Default outbound embedding request budget, in requests per minute.
///
/// Cloud embedding backends cap requests at ~60/min per account. Used when the
/// operator hasn't overridden `memory.embedding_rate_limit_per_min`.
///
/// Keep in sync with `default_embedding_rate_limit_per_min` in
/// `config::schema::storage_memory`.
pub const DEFAULT_EMBEDDING_RATE_LIMIT_PER_MIN: u32 = 60;

/// Process-global configured budget (requests/min). `0` disables throttling.
static CONFIGURED_LIMIT: AtomicU32 = AtomicU32::new(DEFAULT_EMBEDDING_RATE_LIMIT_PER_MIN);

/// Process-global registry of per-endpoint token buckets, keyed by base URL.
static BUCKETS: OnceLock<Mutex<HashMap<String, Arc<TokenBucket>>>> = OnceLock::new();

/// Override the process-global embedding request budget. `0` disables
/// throttling entirely.
///
/// Wired from config load (`config::schema::load::apply_env_overrides`) so the
/// live budget tracks `memory.embedding_rate_limit_per_min`. When the rate
/// changes, existing buckets are dropped so the new rate takes effect on the
/// next request — mirroring how `proxy::set_runtime_proxy_config` clears its
/// client cache on reconfigure.
pub fn set_embedding_rate_limit(per_minute: u32) {
    let prev = CONFIGURED_LIMIT.swap(per_minute, Ordering::Relaxed);
    // Only drop buckets when the rate actually changes. Clearing on every call
    // (e.g. repeated config reloads with an unchanged value) would keep handing
    // out a fresh burst token and erode the hard-cap pacing guarantee.
    if prev != per_minute {
        if let Some(registry) = BUCKETS.get() {
            registry
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
        }
    }
    tracing::debug!(
        target: "embeddings::rate_limit",
        "[embeddings] rate limit set to {per_minute}/min ({})",
        if per_minute == 0 { "disabled" } else { "enabled" }
    );
}

/// The current process-global embedding request budget (requests/min).
#[must_use]
pub fn embedding_rate_limit() -> u32 {
    CONFIGURED_LIMIT.load(Ordering::Relaxed)
}

/// Gate one outbound embedding HTTP request for `base_url`.
///
/// Blocks cooperatively until the per-endpoint token bucket has a slot, then
/// consumes it. No-ops when throttling is disabled (`limit == 0`) or when
/// `base_url` is a loopback host.
pub async fn acquire_embedding_slot(base_url: &str) {
    acquire_with_limit(base_url, embedding_rate_limit()).await;
}

/// Inner of [`acquire_embedding_slot`] with the budget passed explicitly, so
/// tests can exercise the gating logic without mutating the process-global
/// limit (which would race across parallel tests).
async fn acquire_with_limit(base_url: &str, limit: u32) {
    if limit == 0 || is_loopback_url(base_url) {
        return;
    }
    bucket_for(base_url, limit).acquire().await;
}

/// Get-or-create the bucket for `base_url`. The first caller's `per_minute`
/// fixes the rate for that endpoint until [`set_embedding_rate_limit`] clears
/// the registry; in practice the limit is configured once at startup before
/// any embedding request, so the rate is uniform across the process.
fn bucket_for(base_url: &str, per_minute: u32) -> Arc<TokenBucket> {
    let registry = BUCKETS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = registry.lock().unwrap_or_else(PoisonError::into_inner);
    map.entry(base_url.to_string())
        .or_insert_with(|| {
            tracing::debug!(
                target: "embeddings::rate_limit",
                "[embeddings] new rate limiter endpoint={base_url} limit={per_minute}/min"
            );
            Arc::new(TokenBucket::per_minute(per_minute))
        })
        .clone()
}

/// True when `base_url`'s host is loopback (`localhost`, `127.0.0.0/8`, `::1`).
/// Unparseable URLs are treated as non-loopback so a malformed remote endpoint
/// still gets throttled rather than silently bypassing the limiter.
fn is_loopback_url(base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url) else {
        return false;
    };
    match url.host_str() {
        Some(host) => {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .parse::<IpAddr>()
                    .map(|ip| ip.is_loopback())
                    .unwrap_or(false)
        }
        None => false,
    }
}

/// Minimum-interval token bucket sized for a **hard** requests-per-minute cap.
///
/// Capacity is intentionally a single token, not `per_minute`. The backend
/// enforces 60/min as a hard limit, and a token bucket admits up to
/// `capacity + refill × window` requests in any window — so a `per_minute`-sized
/// burst could momentarily reach ~`2 × per_minute` in the first rolling minute
/// and trip the cap. With capacity 1 the bucket paces requests at one per
/// `60 / per_minute` seconds — a steady `per_minute` per minute with no burst.
/// An idle bucket refills that one token, so an occasional lone request (e.g. an
/// interactive retrieval query embed) still goes out immediately; only
/// back-to-back requests are spaced.
/// [`Self::acquire`] consumes one token, sleeping until one accrues if empty.
struct TokenBucket {
    state: tokio::sync::Mutex<BucketState>,
    capacity: f64,
    refill_per_sec: f64,
}

struct BucketState {
    tokens: f64,
    last_refill: Instant,
}

/// Burst allowance, in tokens. One token = no burst beyond a single request,
/// which is what keeps us strictly under a hard per-minute cap.
const BURST_TOKENS: f64 = 1.0;

impl TokenBucket {
    fn per_minute(per_minute: u32) -> Self {
        // `max(1)` is purely defensive against divide-by-zero; the `limit == 0`
        // path in `acquire_embedding_slot` never constructs a bucket.
        let refill_per_sec = f64::from(per_minute.max(1)) / 60.0;
        Self {
            state: tokio::sync::Mutex::new(BucketState {
                tokens: BURST_TOKENS,
                last_refill: Instant::now(),
            }),
            capacity: BURST_TOKENS,
            refill_per_sec,
        }
    }

    async fn acquire(&self) {
        loop {
            // Compute refill + the wait-until-next-token *while holding the
            // lock*, but drop the guard before sleeping so concurrent callers
            // aren't blocked on the mutex during the sleep.
            let wait = {
                let mut state = self.state.lock().await;
                let now = Instant::now();
                let elapsed = now.duration_since(state.last_refill).as_secs_f64();
                state.last_refill = now;
                match refill_and_take(
                    &mut state.tokens,
                    self.capacity,
                    self.refill_per_sec,
                    elapsed,
                ) {
                    None => return,
                    Some(wait) => wait,
                }
            };
            tracing::debug!(
                target: "embeddings::rate_limit",
                "[embeddings] throttling embed request: waiting {:.0}ms for a slot",
                wait.as_secs_f64() * 1000.0
            );
            tokio::time::sleep(wait).await;
        }
    }
}

/// Refill the bucket for `elapsed_secs`, then try to consume one token.
/// Pure in its inputs (no clock) so the rate math is unit-testable directly.
/// Returns `None` when a token was consumed, or `Some(wait)` — the time until
/// the next whole token accrues — when the bucket is dry.
fn refill_and_take(
    tokens: &mut f64,
    capacity: f64,
    refill_per_sec: f64,
    elapsed_secs: f64,
) -> Option<Duration> {
    *tokens = (*tokens + elapsed_secs * refill_per_sec).min(capacity);
    if *tokens >= 1.0 {
        *tokens -= 1.0;
        None
    } else {
        let deficit = 1.0 - *tokens;
        Some(Duration::from_secs_f64(deficit / refill_per_sec))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_hosts_are_exempt() {
        assert!(is_loopback_url("http://localhost:11434"));
        assert!(is_loopback_url("http://LOCALHOST/v1"));
        assert!(is_loopback_url("http://127.0.0.1:1234/v1"));
        assert!(is_loopback_url("http://127.5.6.7"));
        assert!(is_loopback_url("http://[::1]:8080"));
    }

    #[test]
    fn remote_and_malformed_hosts_are_throttled() {
        assert!(!is_loopback_url("https://api.openai.com"));
        assert!(!is_loopback_url("https://api.openhuman.example/openai/v1"));
        assert!(!is_loopback_url("https://10.0.0.5/v1")); // private but not loopback
        assert!(!is_loopback_url("not a url")); // malformed → throttled, not bypassed
    }

    // ── Bucket math (pure, no clock) ─────────────────────────

    #[test]
    fn take_consumes_when_token_available() {
        let mut tokens = 5.0;
        assert!(
            refill_and_take(&mut tokens, 60.0, 1.0, 0.0).is_none(),
            "a token is available → consume, no wait"
        );
        assert!(
            (tokens - 4.0).abs() < 1e-9,
            "one token consumed, got {tokens}"
        );
    }

    #[test]
    fn take_waits_a_full_period_when_empty() {
        let mut tokens = 0.0;
        // Empty bucket at 1 token/sec → wait ~1s, consuming nothing yet.
        let wait = refill_and_take(&mut tokens, 60.0, 1.0, 0.0).expect("must wait");
        assert!((wait.as_secs_f64() - 1.0).abs() < 1e-6, "got {wait:?}");
        assert!(tokens.abs() < 1e-9, "no token consumed while waiting");
    }

    #[test]
    fn partial_refill_shortens_the_wait() {
        let mut tokens = 0.0;
        // 0.25s at 1/sec accrues 0.25 tokens → still <1 → wait the remaining 0.75s.
        let wait = refill_and_take(&mut tokens, 60.0, 1.0, 0.25).expect("must wait");
        assert!((wait.as_secs_f64() - 0.75).abs() < 1e-6, "got {wait:?}");
    }

    #[test]
    fn refill_is_capped_at_capacity() {
        let mut tokens = 50.0;
        // A huge idle gap must not let the bucket overflow capacity.
        assert!(refill_and_take(&mut tokens, 60.0, 1.0, 10_000.0).is_none());
        assert!(
            (tokens - 59.0).abs() < 1e-9,
            "capped at 60 then consumed 1 → 59, got {tokens}"
        );
    }

    // ── Gating glue (explicit limit, no global mutation) ─────

    #[tokio::test]
    async fn disabled_limit_never_blocks() {
        for _ in 0..1000 {
            acquire_with_limit("https://api.example.com/v1", 0).await;
        }
    }

    #[tokio::test]
    async fn loopback_bypasses_even_at_tight_limit() {
        // limit=1 would throttle hard if applied; loopback must bypass it.
        for _ in 0..50 {
            acquire_with_limit("http://127.0.0.1:11434/v1", 1).await;
        }
    }

    #[tokio::test]
    async fn first_remote_call_does_not_block() {
        // Capacity is one token, so a fresh (or idle-refilled) bucket lets
        // exactly one request through immediately; pacing of subsequent
        // back-to-back requests is covered by `acquire_traverses_wait_branch_*`.
        // Unique URL → a bucket isolated from other tests.
        let url = "https://burst-test.example/v1";
        let start = Instant::now();
        acquire_with_limit(url, 60).await;
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "first call on a fresh bucket must not block, elapsed {:?}",
            start.elapsed()
        );
    }

    #[tokio::test]
    async fn back_to_back_acquires_are_paced_no_burst() {
        // The hard-cap guarantee: capacity is one token, so the first acquire
        // passes instantly but the second must wait for refill. 600/min → 10
        // tokens/sec → ~100ms spacing (kept short so the test is fast). A local
        // bucket avoids the global registry (and its clear-on-reconfigure).
        let bucket = TokenBucket::per_minute(600);
        bucket.acquire().await; // consumes the single burst token
        let start = Instant::now();
        bucket.acquire().await; // must be paced
        assert!(
            start.elapsed() >= Duration::from_millis(50),
            "second back-to-back acquire must be paced (no burst), elapsed {:?}",
            start.elapsed()
        );
    }

    #[tokio::test]
    async fn acquire_traverses_wait_branch_when_dry() {
        // Drive the real throttle path (refill → still dry → sleep → re-check →
        // consume) without a slow test: pre-seed the bucket just under one
        // token so the wait is ~0.1s of real time. 60/min == 1 token/sec.
        let bucket = TokenBucket::per_minute(60);
        {
            let mut state = bucket.state.lock().await;
            state.tokens = 0.9;
            state.last_refill = Instant::now();
        }
        let start = Instant::now();
        bucket.acquire().await; // must sleep ~100ms for the final 0.1 token
        assert!(
            start.elapsed() >= Duration::from_millis(50),
            "dry bucket must wait for a token, elapsed {:?}",
            start.elapsed()
        );
    }

    #[tokio::test]
    async fn public_entrypoint_reads_global_and_delegates() {
        // Exercises the read-global + delegate path; loopback returns at once
        // regardless of the configured rate.
        acquire_embedding_slot("http://localhost:11434/v1").await;
    }

    // ── Global limit setter/getter ───────────────────────────

    #[test]
    fn set_and_read_round_trip() {
        set_embedding_rate_limit(0);
        assert_eq!(embedding_rate_limit(), 0);
        set_embedding_rate_limit(120);
        assert_eq!(embedding_rate_limit(), 120);
        set_embedding_rate_limit(DEFAULT_EMBEDDING_RATE_LIMIT_PER_MIN); // restore global
    }
}
