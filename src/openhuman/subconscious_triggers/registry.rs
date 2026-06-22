//! Trigger ingestion front-end: dedupe + per-source rate limiting.
//!
//! This slice provides the pure admission primitives ([`DedupeWindow`],
//! [`RateLimiter`]) and the [`TriggerRegistry`] shell that composes them.
//! Event normalization (`DomainEvent` → [`Trigger`]) is added in slice 2;
//! the gate + queue wiring in later slices.
//!
//! All time is injected as epoch seconds (`now`) so the logic is fully
//! deterministic under test — no wall-clock reads here.

use std::collections::HashMap;
use std::sync::Mutex;

use super::types::Trigger;

/// Result of admitting a trigger past dedupe + rate limiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmitOutcome {
    /// The trigger passed both gates and may proceed to the LLM gate.
    Admitted,
    /// A trigger with the same dedupe key was seen within the TTL window.
    Duplicate,
    /// The per-source rate limit was exhausted.
    RateLimited,
}

/// Collapses triggers sharing a [`DedupeKey`] within a TTL window so a burst
/// of identical webhooks costs at most one gate call.
#[derive(Debug)]
pub struct DedupeWindow {
    ttl_secs: f64,
    seen: Mutex<HashMap<String, f64>>,
}

impl DedupeWindow {
    pub fn new(ttl_secs: f64) -> Self {
        Self {
            ttl_secs: ttl_secs.max(0.0),
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// Return `true` if `key` is fresh (and record it), or `false` if a
    /// non-expired entry already exists. Expired entries are evicted lazily
    /// on each call to keep the map bounded.
    pub fn check_and_record(&self, key: &str, now: f64) -> bool {
        let mut seen = self.seen.lock().expect("dedupe mutex poisoned");
        // Evict expired entries.
        seen.retain(|_, &mut ts| now - ts < self.ttl_secs);

        if let Some(&ts) = seen.get(key) {
            if now - ts < self.ttl_secs {
                return false;
            }
        }
        seen.insert(key.to_string(), now);
        true
    }

    /// Current number of live (un-evicted) entries. Diagnostic aid.
    pub fn live_len(&self, now: f64) -> usize {
        let seen = self.seen.lock().expect("dedupe mutex poisoned");
        seen.values()
            .filter(|&&ts| now - ts < self.ttl_secs)
            .count()
    }
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    last: f64,
}

/// Per-key token-bucket rate limiter. Keyed by trigger source family so a
/// flood from one source can't starve the gate for others.
#[derive(Debug)]
pub struct RateLimiter {
    capacity: f64,
    refill_per_sec: f64,
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl RateLimiter {
    /// `capacity` tokens, refilling at `refill_per_sec`. A bucket starts
    /// full so the first `capacity` triggers per key are always admitted.
    pub fn new(capacity: f64, refill_per_sec: f64) -> Self {
        Self {
            capacity: capacity.max(1.0),
            refill_per_sec: refill_per_sec.max(0.0),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Consume one token for `key`. Returns `true` if a token was available.
    pub fn allow(&self, key: &str, now: f64) -> bool {
        let mut buckets = self.buckets.lock().expect("rate mutex poisoned");
        let bucket = buckets.entry(key.to_string()).or_insert(Bucket {
            tokens: self.capacity,
            last: now,
        });

        // Refill based on elapsed time, capped at capacity.
        let elapsed = (now - bucket.last).max(0.0);
        bucket.tokens = (bucket.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        bucket.last = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Composes dedupe + rate limiting. Normalization and gate/queue dispatch
/// are layered on in later slices.
#[derive(Debug)]
pub struct TriggerRegistry {
    dedupe: DedupeWindow,
    rate: RateLimiter,
}

impl TriggerRegistry {
    pub fn new(dedupe: DedupeWindow, rate: RateLimiter) -> Self {
        Self { dedupe, rate }
    }

    /// Construct with sensible defaults: 5-minute dedupe TTL, and a rate
    /// bucket of 30 triggers refilling at 1/sec per source family.
    pub fn with_defaults() -> Self {
        Self::new(DedupeWindow::new(300.0), RateLimiter::new(30.0, 1.0))
    }

    /// Run a normalized trigger through dedupe then the per-source rate
    /// limit. Dedupe is checked first so duplicates never consume rate
    /// tokens.
    pub fn admit(&self, trigger: &Trigger, now: f64) -> AdmitOutcome {
        if !self
            .dedupe
            .check_and_record(trigger.dedupe_key.as_str(), now)
        {
            return AdmitOutcome::Duplicate;
        }
        if !self.rate.allow(trigger.source.family(), now) {
            return AdmitOutcome::RateLimited;
        }
        AdmitOutcome::Admitted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::subconscious_triggers::types::{
        DedupeKey, TriggerPayload, TriggerPriority, TriggerSource,
    };

    fn trigger(key: &str, family: TriggerSource) -> Trigger {
        Trigger {
            id: "id".into(),
            source: family,
            display_label: key.into(),
            payload: TriggerPayload {
                gate_summary: key.into(),
                raw: serde_json::Value::Null,
            },
            priority: TriggerPriority::Normal,
            dedupe_key: DedupeKey(key.into()),
            external_content: false,
            received_at: 0.0,
        }
    }

    fn cron() -> TriggerSource {
        TriggerSource::Cron {
            job_id: "j".into(),
            job_name: "n".into(),
        }
    }

    // ── DedupeWindow ────────────────────────────────────────────────────

    #[test]
    fn dedupe_collapses_within_ttl() {
        let w = DedupeWindow::new(60.0);
        assert!(w.check_and_record("k", 0.0));
        assert!(!w.check_and_record("k", 30.0)); // within TTL → duplicate
        assert!(!w.check_and_record("k", 59.9));
    }

    #[test]
    fn dedupe_allows_again_after_ttl() {
        let w = DedupeWindow::new(60.0);
        assert!(w.check_and_record("k", 0.0));
        assert!(w.check_and_record("k", 60.1)); // past TTL → fresh again
    }

    #[test]
    fn dedupe_distinct_keys_are_independent() {
        let w = DedupeWindow::new(60.0);
        assert!(w.check_and_record("a", 0.0));
        assert!(w.check_and_record("b", 0.0));
        assert_eq!(w.live_len(0.0), 2);
    }

    #[test]
    fn dedupe_evicts_expired_entries() {
        let w = DedupeWindow::new(10.0);
        w.check_and_record("a", 0.0);
        w.check_and_record("b", 0.0);
        // Touch at t=20 → both expired and evicted; only "c" remains.
        w.check_and_record("c", 20.0);
        assert_eq!(w.live_len(20.0), 1);
    }

    // ── RateLimiter ─────────────────────────────────────────────────────

    #[test]
    fn rate_allows_up_to_capacity_then_blocks() {
        let r = RateLimiter::new(3.0, 0.0); // no refill
        assert!(r.allow("s", 0.0));
        assert!(r.allow("s", 0.0));
        assert!(r.allow("s", 0.0));
        assert!(!r.allow("s", 0.0)); // bucket empty
    }

    #[test]
    fn rate_refills_over_time() {
        let r = RateLimiter::new(2.0, 1.0); // 1 token/sec
        assert!(r.allow("s", 0.0));
        assert!(r.allow("s", 0.0));
        assert!(!r.allow("s", 0.0));
        assert!(r.allow("s", 1.0)); // 1 sec → 1 token back
    }

    #[test]
    fn rate_keys_are_independent() {
        let r = RateLimiter::new(1.0, 0.0);
        assert!(r.allow("a", 0.0));
        assert!(!r.allow("a", 0.0));
        assert!(r.allow("b", 0.0)); // separate bucket
    }

    // ── TriggerRegistry::admit ──────────────────────────────────────────

    #[test]
    fn admit_passes_fresh_trigger() {
        let reg = TriggerRegistry::with_defaults();
        assert_eq!(
            reg.admit(&trigger("k", cron()), 0.0),
            AdmitOutcome::Admitted
        );
    }

    #[test]
    fn admit_rejects_duplicate_before_consuming_rate() {
        let reg = TriggerRegistry::new(DedupeWindow::new(300.0), RateLimiter::new(1.0, 0.0));
        assert_eq!(
            reg.admit(&trigger("k", cron()), 0.0),
            AdmitOutcome::Admitted
        );
        // Same key → Duplicate (must NOT have consumed the single rate token
        // on the first call beyond the one admit, so a *different* key still
        // works within capacity).
        assert_eq!(
            reg.admit(&trigger("k", cron()), 1.0),
            AdmitOutcome::Duplicate
        );
    }

    #[test]
    fn admit_rate_limits_distinct_keys_same_family() {
        let reg = TriggerRegistry::new(DedupeWindow::new(300.0), RateLimiter::new(1.0, 0.0));
        assert_eq!(
            reg.admit(&trigger("a", cron()), 0.0),
            AdmitOutcome::Admitted
        );
        // Distinct dedupe key (passes dedupe) but same family → rate limited.
        assert_eq!(
            reg.admit(&trigger("b", cron()), 0.0),
            AdmitOutcome::RateLimited
        );
    }
}
